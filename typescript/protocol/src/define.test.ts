import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { canonicalizeJcs } from "@usearete/hash";
import { describe, expect, it } from "vitest";

import tokenRaise from "../fixtures/token-raise/protocol.js";
import { defineProtocol, lowerProtocol, parseContractJson, validateContract, type RejectionCode } from "./index.js";

const here = dirname(fileURLToPath(import.meta.url));
const fixture = parseContractJson(
  readFileSync(resolve(here, "../../../test-vectors/protocol/token-raise.protocol-spec.json")),
);
const decoder = new TextDecoder();

describe("defineProtocol", () => {
  it("lowers the token raise fixture to the shared spec exactly", () => {
    const { spec, handlers, handlerExports } = lowerProtocol(tokenRaise);
    expect(decoder.decode(canonicalizeJcs(spec as never))).toBe(decoder.decode(canonicalizeJcs(fixture as never)));
    expect(validateContract("protocol-spec", spec)).toBe(validateContract("protocol-spec", fixture));
    expect(Object.keys(handlers)).toEqual(["createLaunch"]);
    expect(handlerExports).toEqual({ create: "createLaunch" });
  });

  it("types rejection codes from guard names", () => {
    const code: RejectionCode<typeof tokenRaise, "contribute"> = "wallet_cap_exceeded";
    // @ts-expect-error not a guard of contribute
    const wrong: RejectionCode<typeof tokenRaise, "contribute"> = "raise_not_failed";
    expect([code, wrong]).toHaveLength(2);
  });

  it("fills access defaults and omits access on keeper steps", () => {
    const { spec } = lowerProtocol(
      defineProtocol({
        name: "tiny",
        version: "0.1.0",
        programs: { vault: { programId: "Vau1t11111111111111111111111111111111111111", programSpecHash: `arete:h1:program-spec:sha256:${"d".repeat(64)}` } },
        instanceVisibility: "shared",
        roles: { judge: {} },
        machines: {
          instance: { initial: "open", states: { open: "recorded", done: { terminal: true } } },
          participant: { initial: "in", states: { in: "recorded", out: { terminal: true } } },
        },
        steps: {
          open: {
            scope: "create",
            authority: "creator",
            bind: { roles: { judge: "wallet" } },
            build: [{ program: "vault", ix: "open" }],
          },
          decide: {
            scope: "instance",
            authority: { kind: "role", role: "judge" },
            guards: { not_judge: "wallet == instance.roles.judge" },
            transitions: [{ to: "done" }],
            build: [{ program: "vault", ix: "decide" }],
          },
          join: {
            scope: "participant",
            authority: "participant",
            transitions: [{ machine: "participant", to: "out" }],
            build: [{ program: "vault", instruction: "join" }],
          },
          close: { scope: "instance", authority: "keeper", transitions: [{ to: "done" }], build: [{ program: "vault", ix: "close" }] },
        },
      }),
    );
    expect(spec.steps.open!.access).toBe("secret");
    expect(spec.steps.decide!.access).toBe("secret");
    expect(spec.steps.join!.access).toBe("public");
    expect("access" in spec.steps.close!).toBe(false);
    expect(spec.steps.open!.bind).toEqual({ accounts: {}, roles: { judge: "wallet" } });
    expect(spec.handlerAbi).toBeUndefined();
  });

  it("lowers an empty build to a state-only template step", () => {
    const { spec } = lowerProtocol(
      defineProtocol({
        name: "tiny",
        version: "0.1.0",
        programs: { vault: { programId: "Vau1t11111111111111111111111111111111111111", programSpecHash: `arete:h1:program-spec:sha256:${"d".repeat(64)}` } },
        instanceVisibility: "shared",
        roles: { operator: {} },
        machines: { instance: { initial: "open", states: { open: "recorded", closed: { terminal: true } } } },
        steps: {
          open: { scope: "create", authority: "creator", bind: { roles: { operator: "wallet" } }, build: [{ program: "vault", ix: "open" }] },
          close: {
            scope: "instance",
            authority: { kind: "role", role: "operator" },
            guards: { not_operator: "wallet == instance.roles.operator" },
            transitions: [{ from: ["open"], to: "closed" }],
            build: [],
          },
        },
      }),
    );
    expect(spec.steps.close!.build).toEqual({ kind: "template", instructions: [] });
    expect(validateContract("protocol-spec", spec)).toMatch(/^arete:h1:protocol-spec:sha256:/);
  });

  it("lowers remaining accounts after the IDL accounts and refuses signers", () => {
    const tiny = (remainingAccounts: unknown) =>
      defineProtocol({
        name: "tiny",
        version: "0.1.0",
        programs: { vault: { programId: "Vau1t11111111111111111111111111111111111111", programSpecHash: `arete:h1:program-spec:sha256:${"d".repeat(64)}` } },
        params: { type: "object", properties: { quoteMint: { type: "string", format: "pubkey" } }, required: ["quoteMint"], additionalProperties: false },
        instanceVisibility: "shared",
        machines: { instance: { initial: "open", states: { open: "recorded", done: { terminal: true } } } },
        steps: {
          open: { scope: "create", authority: "creator", bind: { accounts: { pool: "wallet" } }, build: [{ program: "vault", ix: "open" }] },
          close: {
            scope: "instance",
            authority: "keeper",
            transitions: [{ to: "done" }],
            build: [{ program: "vault", ix: "close", remainingAccounts: remainingAccounts as never }],
          },
        },
      });
    const { spec } = lowerProtocol(
      tiny([{ pubkey: "instance.accounts.pool", isWritable: true }, { pubkey: "params.quoteMint" }]),
    );
    const build = spec.steps.close!.build;
    expect(build.kind === "template" && build.instructions[0]!.remainingAccounts).toEqual([
      { pubkey: "instance.accounts.pool", isWritable: true, isSigner: false },
      { pubkey: "params.quoteMint", isWritable: false, isSigner: false },
    ]);
    const empty = lowerProtocol(tiny([])).spec.steps.close!.build;
    expect(empty.kind === "template" && "remainingAccounts" in empty.instructions[0]!).toBe(false);
    expect(() => lowerProtocol(tiny([{ pubkey: "wallet", isSigner: true }]))).toThrow(
      expect.objectContaining({ path: "/steps/close/build/0/remainingAccounts/0" }),
    );
  });

  it("lowers a Squads vault index and omits vault 0", () => {
    const withVault = (vaultIndex: number) =>
      defineProtocol({
        ...tokenRaise,
        steps: {
          ...tokenRaise.steps,
          finalize: {
            ...tokenRaise.steps.finalize,
            authority: { kind: "squadsProposer", multisig: "resources.treasury.multisig", vaultIndex },
          },
        },
      });
    const vectors = parseContractJson(readFileSync(resolve(here, "../../../test-vectors/protocol-v1.json"))) as {
      documents: Record<string, { document: unknown }>;
    };
    const vault = lowerProtocol(withVault(3)).spec;
    expect(decoder.decode(canonicalizeJcs(vault as never))).toBe(
      decoder.decode(canonicalizeJcs(vectors.documents.tokenRaiseVaultIndex!.document as never)),
    );
    const zero = lowerProtocol(withVault(0)).spec;
    expect(zero.steps.finalize!.authority).toEqual({ kind: "squadsProposer", multisig: "resources.treasury.multisig" });
    expect(validateContract("protocol-spec", zero)).toBe(validateContract("protocol-spec", fixture));
    for (const bad of [256, -1, 1.5]) {
      expect(() => lowerProtocol(withVault(bad))).toThrow(
        expect.objectContaining({ path: "/steps/finalize/authority/vaultIndex" }),
      );
    }
  });

  it("rejects a misspelled Squads vault index", () => {
    const typo = defineProtocol({
      ...tokenRaise,
      steps: {
        ...tokenRaise.steps,
        finalize: {
          ...tokenRaise.steps.finalize,
          authority: { kind: "squadsProposer", multisig: "resources.treasury.multisig", vaultindex: 3 } as never,
        },
      },
    });
    expect(() => lowerProtocol(typo)).toThrow(/unknown field .vaultindex./);
  });

  it("reports contract errors with codes and paths", () => {
    expect(() =>
      lowerProtocol(
        defineProtocol({
          name: "bad",
          version: "0.1.0",
          instanceVisibility: "shared",
          machines: { instance: { initial: "open", states: { open: { terminal: true } } } },
          steps: { open: { scope: "create", authority: "creator", build: [{ program: "missing", ix: "x" }] } },
        }),
      ),
    ).toThrow(expect.objectContaining({ code: "unknown-program", path: "/steps/open/build/instructions/0/program" }));
  });
});
