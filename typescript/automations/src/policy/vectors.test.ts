// Runs the shared signer-policy vectors (test-vectors/signer-policy/). The
// Rust MCP signer generates them, so passing them means this policy decides
// exactly as that signer does.

import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

import { checkInner, checkTopLevel } from './authority';
import {
  decodeTransaction,
  transactionPrograms,
  transactionSigners,
  writableStaticKeys,
} from './decode';
import { decodeBase64, encodeBase64, field } from './encoding';
import { checkPrepared, checkTarget, definePolicy, type SignerPolicy } from './policy';
import type { Simulator } from './simulator';
import { summarizePrepared } from './summary';

interface VectorCase {
  name: string;
  input: any;
  expect: any;
  compareReason?: boolean;
}

function load(file: string): VectorCase[] {
  const url = new URL(`../../../../test-vectors/signer-policy/${file}`, import.meta.url);
  const document = JSON.parse(readFileSync(url, 'utf8')) as { version: number; cases: VectorCase[] };
  expect(document.version).toBe(1);
  return document.cases;
}

/** JSON as the reference writes it: drop undefined fields, bigints as strings. */
function json(value: unknown): unknown {
  return JSON.parse(JSON.stringify(value, (_key, item) => (typeof item === 'bigint' ? item.toString() : item)));
}

function policyJson(policy: SignerPolicy): unknown {
  return {
    deployments: policy.deployments,
    maxLamportsPerTransaction: policy.maxLamportsPerTransaction.toString(),
    programs: policy.programs,
    maxTokenAmounts:
      policy.maxTokenAmounts === null
        ? null
        : Object.fromEntries([...policy.maxTokenAmounts].map(([mint, cap]) => [mint, cap.toString()])),
    clusters: policy.clusters,
    clustersListed: policy.clustersListed,
  };
}

/** The chain fixture the vectors describe (see test-vectors/signer-policy/README.md). */
function fixtureSimulator(chain: any): Simulator {
  return {
    async call(method, params) {
      if (method === 'getMultipleAccounts') {
        const addresses = params[0] as string[];
        return {
          context: { slot: chain.slot },
          value: addresses.map((address) => chain.accounts[address] ?? null),
        };
      }
      if (method === 'simulateTransaction') {
        const simulation = chain.simulation;
        if (typeof simulation.rpcError === 'string') {
          throw new Error(`${method} failed: ${simulation.rpcError}`);
        }
        const addresses = (field(field(params[1], 'accounts'), 'addresses') ?? []) as string[];
        const value: Record<string, unknown> = {
          err: simulation.err ?? null,
          logs: simulation.logs ?? [],
          accounts: addresses.map((address) => simulation.accounts?.[address] ?? null),
        };
        if ('innerInstructions' in simulation) value.innerInstructions = simulation.innerInstructions;
        return { context: { slot: simulation.slot }, value };
      }
      throw new Error(`unexpected method ${method}`);
    },
  };
}

async function summary(input: any) {
  const simulator = input.chain === null ? null : fixtureSimulator(input.chain);
  const result = json(await summarizePrepared(input.prepared, input.signer, simulator)) as any;
  for (const entry of result.transactions) {
    if (entry.signerEffect.error !== undefined) entry.signerEffect.error = 'rpc_error';
  }
  return result;
}

describe('signer-policy vectors: config', () => {
  for (const vector of load('config.json')) {
    it(vector.name, () => {
      if (vector.expect.error) {
        expect(() => definePolicy(vector.input)).toThrow();
      } else {
        expect(json(policyJson(definePolicy(vector.input)))).toEqual(vector.expect.policy);
      }
    });
  }
});

describe('signer-policy vectors: target', () => {
  for (const vector of load('target.json')) {
    it(vector.name, () => {
      const { policy, deploymentId, cluster } = vector.input;
      expect(checkTarget(definePolicy(policy), deploymentId, cluster)).toEqual(vector.expect);
    });
  }
});

describe('signer-policy vectors: decode', () => {
  for (const vector of load('decode.json')) {
    it(vector.name, () => {
      const bytes = decodeBase64(vector.input.transactionBase64)!;
      if (vector.expect.error) {
        expect(() => decodeTransaction(bytes)).toThrow();
        return;
      }
      const decoded = decodeTransaction(bytes);
      expect({
        version: decoded.version,
        requiredSignatures: decoded.requiredSignatures,
        staticKeys: decoded.staticKeys,
        writableStatic: decoded.writableStatic,
        writableStaticKeys: writableStaticKeys(decoded),
        instructions: decoded.instructions.map((instruction) => ({
          programIndex: instruction.programIndex,
          accounts: instruction.accounts,
          data: encodeBase64(instruction.data),
        })),
        lookups: decoded.lookups,
        message: [decoded.message.start, decoded.message.end],
        signatureOffset: decoded.signatureOffset,
        signers: transactionSigners(decoded).map(([key, signed]) => ({ key, signed })),
        programs: transactionPrograms(decoded),
      }).toEqual(vector.expect);
    });
  }
});

describe('signer-policy vectors: authority', () => {
  for (const vector of load('authority.json')) {
    it(vector.name, () => {
      const { input } = vector;
      const checked =
        input.kind === 'topLevel'
          ? checkTopLevel(
              decodeTransaction(decodeBase64(input.transactionBase64)!),
              input.signer,
              input.lookupKeys,
            )
          : checkInner(input.innerInstructions, input.signer, input.accountKeys);
      expect(json(checked)).toEqual(vector.expect);
    });
  }
});

describe('signer-policy vectors: summary', () => {
  for (const vector of load('summary.json')) {
    it(vector.name, async () => {
      expect(await summary(vector.input)).toEqual(vector.expect);
    });
  }
});

describe('signer-policy vectors: check', () => {
  for (const vector of load('check.json')) {
    it(vector.name, async () => {
      const { input } = vector;
      const simulator = input.chain === null ? null : fixtureSimulator(input.chain);
      const prepared = await summarizePrepared(input.prepared, input.signer, simulator);
      const refusal = checkPrepared(definePolicy(input.policy), {
        deploymentId: input.deploymentId,
        prepared: input.prepared,
        summary: prepared,
        simulates: simulator !== null,
      });
      if (vector.expect === null || vector.compareReason !== false) {
        expect(refusal).toEqual(vector.expect);
      } else {
        expect(refusal?.rule).toBe(vector.expect.rule);
      }
    });
  }
});
