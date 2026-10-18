/**
 * Pure build artifacts derived from a validated spec: the machine map and
 * step anatomy for UIs, the release, the compiler identity and the build
 * receipt. Nothing here touches the filesystem, so it runs in browsers too.
 */

import { sha256 } from "@noble/hashes/sha256";
import { bytesToHex } from "@noble/hashes/utils";
import {
  canonicalizeJcs,
  createCompilerV1,
  hashCompilerV1,
  hashJcs,
  type CompilerHash,
  type ProtocolHandlerBundleHash,
  type ProtocolReleaseHash,
  type ProtocolSpecHash,
  type ProtocolStackSourceHash,
} from "@usearete/hash";

import { hashHandlerBundle } from "../contracts/index.js";
import { compareUtf8, sortedKeys } from "../contracts/shape.js";
import { machineEdges } from "../contracts/spec.js";
import {
  PROTOCOL_BUILD_RECEIPT_SCHEMA_V1,
  PROTOCOL_RELEASE_SCHEMA_V1,
  type AuthorityV1,
  type BuildDependencyV1,
  type MachineKind,
  type ProtocolBuildReceiptV1,
  type ProtocolReleaseV1,
  type ProtocolSpecV1,
  type ProtocolStackSourceV1,
  type StepAccess,
  type StepScope,
} from "../contracts/types.js";
import { generateStackSource, type GenerateStackSourceOptions } from "../stack/generate.js";
import { PACKAGE_NAME, PACKAGE_VERSION } from "../version.js";

const encoder = new TextEncoder();

// ---------------------------------------------------------------------------
// Machine map
// ---------------------------------------------------------------------------

export interface MachineMapEdgeV1 {
  from: string;
  to: string;
  via: { step: string } | { derive: number };
}

export interface MachineMapMachineV1 {
  initial: string;
  states: Array<{ name: string; kind: "recorded" | "derived"; terminal: boolean }>;
  edges: MachineMapEdgeV1[];
}

export interface MachineMapV1 {
  machines: { instance: MachineMapMachineV1; participant?: MachineMapMachineV1 };
  steps: Array<{
    name: string;
    scope: StepScope;
    authority: AuthorityV1;
    access: StepAccess | null;
    guards: string[];
  }>;
}

function machineMap(spec: ProtocolSpecV1, kind: MachineKind): MachineMapMachineV1 | undefined {
  const machine = kind === "instance" ? spec.machines.instance : spec.machines.participant;
  if (machine === undefined) return undefined;
  const edges: MachineMapEdgeV1[] = [];
  for (const edge of machineEdges(spec, kind)) {
    for (const from of edge.from) {
      if (from !== edge.to) edges.push({ from, to: edge.to, via: edge.via });
    }
  }
  return {
    initial: machine.initial,
    states: sortedKeys(machine.states).map((name) => ({
      name,
      kind: machine.states[name]!.kind,
      terminal: machine.states[name]!.terminal,
    })),
    edges,
  };
}

/** States, edges (one per source state) and steps, for UIs that draw the protocol. */
export function buildMachineMap(spec: ProtocolSpecV1): MachineMapV1 {
  const participant = machineMap(spec, "participant");
  return {
    machines: { instance: machineMap(spec, "instance")!, ...(participant ? { participant } : {}) },
    steps: sortedKeys(spec.steps).map((name) => {
      const step = spec.steps[name]!;
      return {
        name,
        scope: step.scope,
        authority: step.authority,
        access: step.access ?? null,
        guards: step.guards.map((guard) => guard.code),
      };
    }),
  };
}

// ---------------------------------------------------------------------------
// Step anatomy
// ---------------------------------------------------------------------------

/**
 * One instruction slot in a step's transaction, in order. `program` slots
 * come from the step's template or handler; `glue` and `runtime` slots are
 * placeholders the platform fills when it compiles the transaction.
 */
export type AnatomySlotV1 =
  | { role: "runtime"; kind: "computeBudget" }
  | { role: "glue"; kind: "createAta"; owner: string; mint: string }
  | { role: "glue"; kind: "wrapSol"; amount: string }
  | { role: "runtime"; kind: "fee"; basis: "input" | "fixed"; placement: "before" | "policy" }
  | { role: "program"; kind: "template"; program: string; instruction: string }
  | { role: "program"; kind: "handler"; export: string; programs: string[] }
  | { role: "glue"; kind: "unwrapSol" }
  | { role: "runtime"; kind: "memo" };

export interface StepAnatomyV1 {
  scope: StepScope;
  authority: AuthorityV1;
  instructions: AnatomySlotV1[];
}

/** The instruction layout of every step, with platform-injected slots marked. */
export function buildAnatomy(spec: ProtocolSpecV1): { steps: Record<string, StepAnatomyV1> } {
  const steps: Record<string, StepAnatomyV1> = {};
  for (const name of sortedKeys(spec.steps)) {
    const step = spec.steps[name]!;
    const slots: AnatomySlotV1[] = [{ role: "runtime", kind: "computeBudget" }];
    for (const ata of step.glue.atas) slots.push({ role: "glue", kind: "createAta", owner: ata.owner, mint: ata.mint });
    if (step.glue.wrapSol !== undefined) slots.push({ role: "glue", kind: "wrapSol", amount: step.glue.wrapSol.amount });
    if (step.fee.kind === "input") slots.push({ role: "runtime", kind: "fee", basis: "input", placement: "before" });
    // A fixed fee's placement comes from the revision's fee policy.
    if (step.fee.kind === "fixed") slots.push({ role: "runtime", kind: "fee", basis: "fixed", placement: "policy" });
    if (step.build.kind === "template") {
      for (const instruction of step.build.instructions) {
        slots.push({ role: "program", kind: "template", program: instruction.program, instruction: instruction.instruction });
      }
    } else {
      slots.push({ role: "program", kind: "handler", export: step.build.export, programs: [...step.build.programs] });
    }
    if (step.glue.unwrapSol) slots.push({ role: "glue", kind: "unwrapSol" });
    slots.push({ role: "runtime", kind: "memo" });
    steps[name] = { scope: step.scope, authority: step.authority, instructions: slots };
  }
  return { steps };
}

// ---------------------------------------------------------------------------
// Release, compiler identity and receipt
// ---------------------------------------------------------------------------

export interface CompilerIdentity {
  name: string;
  version: string;
  esbuild: string;
}

export function compilerIdentity(esbuildVersion: string): CompilerIdentity {
  return { name: PACKAGE_NAME, version: PACKAGE_VERSION, esbuild: esbuildVersion };
}

/**
 * The `compiler` hash (`arete.compiler/v1`, framed-tuple): one source per
 * identity field, so a new package or esbuild version is a new compiler.
 */
export function compilerHash(identity: CompilerIdentity): CompilerHash {
  return hashCompilerV1(
    createCompilerV1([
      { path: "esbuild", bytes: encoder.encode(identity.esbuild) },
      { path: "name", bytes: encoder.encode(identity.name) },
      { path: "version", bytes: encoder.encode(identity.version) },
    ]),
  );
}

/** Release dependencies: every ProgramSpec program and stack, sorted by alias; builtins excluded. */
export function releaseDependencies(spec: ProtocolSpecV1): ProtocolReleaseV1["dependencies"] {
  const programs = Object.keys(spec.programs)
    .sort(compareUtf8)
    .flatMap((alias) => {
      const program = spec.programs[alias]!;
      if (program.kind !== "programSpec") return [];
      return [
        {
          alias,
          programId: program.programId,
          programSpecHash: program.programSpecHash,
          ...(program.programReleaseHash !== undefined ? { programReleaseHash: program.programReleaseHash } : {}),
        },
      ];
    });
  const stacks = Object.keys(spec.stacks)
    .sort(compareUtf8)
    .map((alias) => ({ alias, stackManifestHash: spec.stacks[alias]!.stackManifestHash }));
  return { programs, stacks, sdkExtensions: [] };
}

export function buildRelease(
  spec: ProtocolSpecV1,
  hashes: {
    specHash: ProtocolSpecHash;
    handlerBundleHash: ProtocolHandlerBundleHash | null;
    stackSourceHash: ProtocolStackSourceHash;
    compilerHash: CompilerHash;
  },
): ProtocolReleaseV1 {
  return {
    schema: PROTOCOL_RELEASE_SCHEMA_V1,
    protocol: spec.name,
    version: spec.version,
    specHash: hashes.specHash,
    handlerBundleHash: hashes.handlerBundleHash,
    stackSourceHash: hashes.stackSourceHash,
    compilerHash: hashes.compilerHash,
    dependencies: releaseDependencies(spec),
  };
}

export function buildReceipt(input: {
  sourceTreeSha256: string;
  lockfileSha256: string;
  dependencies: BuildDependencyV1[];
  compiler: CompilerIdentity;
  outputs: {
    specHash: ProtocolSpecHash;
    handlerBundleHash: ProtocolHandlerBundleHash | null;
    stackSourceHash: ProtocolStackSourceHash;
    releaseHash: ProtocolReleaseHash;
  };
}): ProtocolBuildReceiptV1 {
  return {
    schema: PROTOCOL_BUILD_RECEIPT_SCHEMA_V1,
    sourceTreeSha256: input.sourceTreeSha256,
    lockfileSha256: input.lockfileSha256,
    dependencies: [...input.dependencies].sort((a, b) => compareUtf8(a.name, b.name)),
    compiler: { ...input.compiler },
    outputs: { ...input.outputs },
  };
}

// ---------------------------------------------------------------------------
// Server-derived releases
// ---------------------------------------------------------------------------

/** The compiler and stack-generator name of server-side derivation. */
export const SERVER_COMPILER_NAME = "arete-protocol-server";

/**
 * The version of server-side derivation. It changes only when the derived
 * documents for the same inputs change, so re-uploading the same spec stays
 * idempotent across server upgrades. Mirrors `SERVER_DERIVATION_VERSION` in
 * `arete-hash`.
 */
export const SERVER_DERIVATION_VERSION = "1.0.0";

/** The server never bundles; the receipt contract needs a semver, so `0.0.0`. */
export const SERVER_COMPILER_ESBUILD = "0.0.0";

/** The compiler identity a hosted service uses when it derives a release from an upload. */
export function serverCompilerIdentity(): CompilerIdentity {
  return { name: SERVER_COMPILER_NAME, version: SERVER_DERIVATION_VERSION, esbuild: SERVER_COMPILER_ESBUILD };
}

/**
 * Stack-source options of server-side derivation: the interim profile, the
 * generator's default program sources and the server generator identity.
 */
export function serverStackSourceOptions(): GenerateStackSourceOptions {
  return { generator: { name: SERVER_COMPILER_NAME, version: SERVER_DERIVATION_VERSION } };
}

export interface ServerDerivedRelease {
  stackSource: ProtocolStackSourceV1;
  release: ProtocolReleaseV1;
  specHash: ProtocolSpecHash;
  handlerBundleHash: ProtocolHandlerBundleHash | null;
  stackSourceHash: ProtocolStackSourceHash;
  releaseHash: ProtocolReleaseHash;
  receipt: ProtocolBuildReceiptV1;
}

const EMPTY_SHA256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/**
 * What a hosted service derives from an upload of `protocol-spec.json` (and
 * `handlers.mjs` when a step has a handler): the interim stack source, the
 * release and a receipt, under `serverCompilerIdentity()`. The same as
 * `derive_server_release` in `arete-hash`.
 *
 * A local build's release hash differs from this one only through the
 * compiler identity and the stack source (server generator identity and
 * default program sources).
 */
export function deriveServerRelease(spec: ProtocolSpecV1, handlerBundle: Uint8Array | null): ServerDerivedRelease {
  const hasHandlers = Object.values(spec.steps).some((step) => step.build.kind === "handler");
  if (hasHandlers && handlerBundle === null) {
    throw new Error("the spec has handler steps but no handler bundle was supplied");
  }
  if (!hasHandlers && handlerBundle !== null) {
    throw new Error("a template-only spec cannot carry a handler bundle");
  }
  if (handlerBundle !== null && handlerBundle.length === 0) throw new Error("the handler bundle is empty");
  const identity = serverCompilerIdentity();
  const stackSource = generateStackSource(spec, serverStackSourceOptions());
  const specHash = hashJcs("protocol-spec", spec as never);
  const handlerBundleHash = handlerBundle === null ? null : hashHandlerBundle(handlerBundle);
  const stackSourceHash = hashJcs("protocol-stack-source", stackSource as never);
  const release = buildRelease(spec, { specHash, handlerBundleHash, stackSourceHash, compilerHash: compilerHash(identity) });
  const releaseHash = hashJcs("protocol-release", release as never);
  const hex = (bytes: Uint8Array): string => bytesToHex(sha256(bytes));
  const tree: Array<[string, string]> = [];
  if (handlerBundle !== null) tree.push(["handlers.mjs", hex(handlerBundle)]);
  tree.push(["protocol-spec.json", hex(canonicalizeJcs(spec as never))]);
  const receipt = buildReceipt({
    sourceTreeSha256: hex(canonicalizeJcs(tree as never)),
    lockfileSha256: EMPTY_SHA256,
    dependencies: [],
    compiler: identity,
    outputs: { specHash, handlerBundleHash, stackSourceHash, releaseHash },
  });
  return { stackSource, release, specHash, handlerBundleHash, stackSourceHash, releaseHash, receipt };
}
