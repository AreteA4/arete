import {
  hashJcs,
  hashRawBytes,
  parseJsonBytesStrict,
} from "./canonical.js";
import { hashProgramSpecV1, validateProgramSpecV1, type ProgramSpecV1 } from "./idl.js";
import { parseHashId } from "./hash.js";
import type {
  ArtifactFileHash,
  JsonValue,
  LiveSpecHash,
  ProgramSpecHash,
  StackManifestHash,
} from "./types.js";

export const ARTIFACT_VERSION_V1 = "1.0.0" as const;
export const LIVE_SPEC_SCHEMA_V1 = "arete.live-spec/v1" as const;
export const LIVE_SPEC_SCHEMA_V2 = "arete.live-spec/v2" as const;
export const STACK_MANIFEST_SCHEMA_V1 = "arete.stack-manifest/v1" as const;
export const STACK_MANIFEST_SCHEMA_V2 = "arete.stack-manifest/v2" as const;

export interface ProgramSpecArtifact {
  readonly artifactVersion: string;
  readonly kind: "program-spec";
  readonly artifactHash: ProgramSpecHash;
  readonly payload: ProgramSpecV1;
}

export interface ProgramRequirementV1 {
  readonly programId: string;
  readonly programSpecHash: ProgramSpecHash;
}

export interface LegacyProgramExtensionsV1 {
  readonly pdas?: Readonly<Record<string, JsonValue>>;
  readonly instructions?: readonly JsonValue[];
}

export interface LiveSpecV1 {
  readonly schema: typeof LIVE_SPEC_SCHEMA_V1;
  readonly compilerContractVersion: string;
  readonly wireContractVersion: string;
  readonly programs: readonly ProgramRequirementV1[];
  readonly entities: readonly JsonValue[];
  readonly legacyProgramExtensions?: LegacyProgramExtensionsV1;
}

export interface LiveSpecV2 {
  readonly schema: typeof LIVE_SPEC_SCHEMA_V2;
  readonly compilerContractVersion: string;
  readonly wireContractVersion: string;
  readonly programs: readonly ProgramRequirementV1[];
  readonly entities: readonly JsonValue[];
  readonly programAdapters: readonly JsonValue[];
}

export interface LiveSpecArtifact<Payload extends LiveSpecV1 | LiveSpecV2 = LiveSpecV1 | LiveSpecV2> {
  readonly artifactVersion: string;
  readonly kind: "live-spec";
  readonly artifactHash: LiveSpecHash;
  readonly payload: Payload;
}

export interface ProgramSpecReferenceV1 {
  readonly programId: string;
  readonly artifactHash: ProgramSpecHash;
}

export interface LiveSpecReferenceV1 {
  readonly artifactHash: LiveSpecHash;
}

export interface SelectedViewV1 {
  readonly liveSpecHash: LiveSpecHash;
  readonly viewId: string;
}

export interface StackManifestV1 {
  readonly schema: typeof STACK_MANIFEST_SCHEMA_V1;
  readonly name: string;
  readonly programs: readonly ProgramSpecReferenceV1[];
  readonly liveSpecs: readonly LiveSpecReferenceV1[];
  readonly selectedViews: readonly SelectedViewV1[];
  readonly queries?: readonly JsonValue[];
  readonly extensions?: Readonly<Record<string, JsonValue>>;
  readonly metadata?: Readonly<Record<string, JsonValue>>;
}

export interface LiveSpecReferenceV2 {
  readonly alias: string;
  readonly artifactHash: LiveSpecHash;
}

export interface SelectedViewV2 {
  readonly liveAlias: string;
  readonly viewId: string;
}

export interface StackManifestV2 {
  readonly schema: typeof STACK_MANIFEST_SCHEMA_V2;
  readonly name: string;
  readonly programs: readonly ProgramSpecReferenceV1[];
  readonly liveSpecs: readonly LiveSpecReferenceV2[];
  readonly selectedViews: readonly SelectedViewV2[];
  readonly queries?: readonly JsonValue[];
  readonly extensions?: Readonly<Record<string, JsonValue>>;
  readonly metadata?: Readonly<Record<string, JsonValue>>;
}

export interface StackManifestArtifact<
  Payload extends StackManifestV1 | StackManifestV2 = StackManifestV1 | StackManifestV2,
> {
  readonly artifactVersion: string;
  readonly kind: "stack-manifest";
  readonly artifactHash: StackManifestHash;
  readonly payload: Payload;
}

export interface LoadedArtifact<A> {
  readonly artifact: A;
  readonly originalBytes: Uint8Array;
  readonly sourceHash: ArtifactFileHash;
}

export function createProgramSpecArtifact(payload: ProgramSpecV1): ProgramSpecArtifact {
  validateProgramSpecV1(payload);
  return {
    artifactVersion: ARTIFACT_VERSION_V1,
    kind: "program-spec",
    artifactHash: hashProgramSpecV1(payload),
    payload,
  };
}

export function createLiveSpecArtifact(payload: LiveSpecV1): LiveSpecArtifact<LiveSpecV1> {
  validateLiveSpec(payload);
  const projection = {
    artifactVersion: ARTIFACT_VERSION_V1,
    kind: "live-spec",
    payload,
  } as unknown as JsonValue;
  return {
    artifactVersion: ARTIFACT_VERSION_V1,
    kind: "live-spec",
    artifactHash: hashJcs("live-spec", projection),
    payload,
  };
}

export function createStackManifestArtifact(
  payload: StackManifestV1,
): StackManifestArtifact<StackManifestV1> {
  validateStackManifest(payload);
  const projection = {
    artifactVersion: ARTIFACT_VERSION_V1,
    kind: "stack-manifest",
    payload,
  } as unknown as JsonValue;
  return {
    artifactVersion: ARTIFACT_VERSION_V1,
    kind: "stack-manifest",
    artifactHash: hashJcs("stack-manifest", projection),
    payload,
  };
}

export function validateProgramSpecArtifact(artifact: ProgramSpecArtifact): void {
  validateVersion(artifact.artifactVersion, "program-spec");
  if (artifact.kind !== "program-spec") throw new Error("expected program-spec artifact");
  parseHashId(artifact.artifactHash, "program-spec");
  validateProgramSpecV1(artifact.payload);
  if (artifact.artifactHash !== hashProgramSpecV1(artifact.payload)) {
    throw new Error("program-spec artifact hash mismatch");
  }
  rejectPrivateFields(artifact as unknown as JsonValue);
}

export function validateLiveSpecArtifact(artifact: LiveSpecArtifact): void {
  validateVersion(artifact.artifactVersion, "live-spec");
  if (artifact.kind !== "live-spec") throw new Error("expected live-spec artifact");
  parseHashId(artifact.artifactHash, "live-spec");
  validateLiveSpec(artifact.payload);
  const expected = hashJcs("live-spec", {
    artifactVersion: artifact.artifactVersion,
    kind: artifact.kind,
    payload: artifact.payload,
  } as unknown as JsonValue);
  if (artifact.artifactHash !== expected) throw new Error("live-spec artifact hash mismatch");
  rejectPrivateFields(artifact as unknown as JsonValue);
}

export function validateStackManifestArtifact(artifact: StackManifestArtifact): void {
  validateVersion(artifact.artifactVersion, "stack-manifest");
  if (artifact.kind !== "stack-manifest") throw new Error("expected stack-manifest artifact");
  parseHashId(artifact.artifactHash, "stack-manifest");
  validateStackManifest(artifact.payload);
  const expected = hashJcs("stack-manifest", {
    artifactVersion: artifact.artifactVersion,
    kind: artifact.kind,
    payload: artifact.payload,
  } as unknown as JsonValue);
  if (artifact.artifactHash !== expected) {
    throw new Error("stack-manifest artifact hash mismatch");
  }
  rejectPrivateFields(artifact as unknown as JsonValue);
}

export function loadProgramSpec(bytes: Uint8Array): LoadedArtifact<ProgramSpecArtifact> {
  const artifact = parseJsonBytesStrict(bytes) as unknown as ProgramSpecArtifact;
  validateProgramSpecArtifact(artifact);
  return { artifact, originalBytes: bytes.slice(), sourceHash: hashRawBytes("artifact-file", bytes) };
}

export function loadLiveSpec(bytes: Uint8Array): LoadedArtifact<LiveSpecArtifact> {
  const artifact = parseJsonBytesStrict(bytes) as unknown as LiveSpecArtifact;
  validateLiveSpecArtifact(artifact);
  return { artifact, originalBytes: bytes.slice(), sourceHash: hashRawBytes("artifact-file", bytes) };
}

export function loadStackManifest(
  bytes: Uint8Array,
): LoadedArtifact<StackManifestArtifact> {
  const artifact = parseJsonBytesStrict(bytes) as unknown as StackManifestArtifact;
  validateStackManifestArtifact(artifact);
  return { artifact, originalBytes: bytes.slice(), sourceHash: hashRawBytes("artifact-file", bytes) };
}

function validateLiveSpec(payload: LiveSpecV1 | LiveSpecV2): void {
  if (payload.schema !== LIVE_SPEC_SCHEMA_V1 && payload.schema !== LIVE_SPEC_SCHEMA_V2) {
    throw new Error(`unsupported live-spec schema '${(payload as { schema: string }).schema}'`);
  }
  if (!payload.compilerContractVersion || !payload.wireContractVersion) {
    throw new Error("live-spec contract versions must not be empty");
  }
  const hashes = new Set<string>();
  for (const program of payload.programs) {
    if (!program.programId) throw new Error("live-spec program ID must not be empty");
    parseHashId(program.programSpecHash, "program-spec");
    if (hashes.has(program.programSpecHash)) throw new Error("duplicate live-spec ProgramSpec");
    hashes.add(program.programSpecHash);
  }
  rejectPrivateFields(payload as unknown as JsonValue);
}

function validateStackManifest(payload: StackManifestV1 | StackManifestV2): void {
  if (payload.schema !== STACK_MANIFEST_SCHEMA_V1 && payload.schema !== STACK_MANIFEST_SCHEMA_V2) {
    throw new Error(
      `unsupported stack-manifest schema '${(payload as { schema: string }).schema}'`,
    );
  }
  if (!payload.name) throw new Error("stack-manifest name must not be empty");
  const programHashes = new Set<string>();
  for (const program of payload.programs) {
    parseHashId(program.artifactHash, "program-spec");
    if (!program.programId || programHashes.has(program.artifactHash)) {
      throw new Error("stack-manifest ProgramSpecs must have unique hashes and program IDs");
    }
    programHashes.add(program.artifactHash);
  }
  if (payload.schema === STACK_MANIFEST_SCHEMA_V2) {
    const aliases = new Set<string>();
    for (const live of payload.liveSpecs) {
      parseHashId(live.artifactHash, "live-spec");
      if (!/^(?=.{1,64}$)(?=.*[A-Za-z0-9])[A-Za-z0-9_-]+$/.test(live.alias)
        || aliases.has(live.alias)) {
        throw new Error("stack-manifest LiveSpecs must have unique portable aliases");
      }
      aliases.add(live.alias);
    }
    const selected = new Set<string>();
    for (const view of payload.selectedViews) {
      const key = `${view.liveAlias}\0${view.viewId}`;
      if (!aliases.has(view.liveAlias) || !view.viewId || selected.has(key)) {
        throw new Error("stack-manifest selected views must reference unique LiveSpec aliases");
      }
      selected.add(key);
    }
  } else {
    for (const live of payload.liveSpecs) parseHashId(live.artifactHash, "live-spec");
  }
  rejectPrivateFields(payload as unknown as JsonValue);
}

function validateVersion(version: string, kind: string): void {
  if (!/^1\.\d+\.\d+$/.test(version)) throw new Error(`unsupported ${kind} version '${version}'`);
}

function rejectPrivateFields(value: JsonValue): void {
  const forbidden = new Set([
    "platform_parser",
    "platformParser",
    "platform_decoder_bundle",
    "platformDecoderBundle",
    "augmented_specs",
    "augmentedSpecs",
    "decoder_binding_id",
    "decoderBindingId",
    "decoder_content_hash",
    "decoderContentHash",
    "artifact_ref",
    "artifactRef",
  ]);
  if (Array.isArray(value)) {
    for (const nested of value) rejectPrivateFields(nested);
    return;
  }
  if (value !== null && typeof value === "object") {
    for (const [key, nested] of Object.entries(value)) {
      if (forbidden.has(key)) throw new Error(`public artifact contains private field '${key}'`);
      rejectPrivateFields(nested);
    }
  }
}
