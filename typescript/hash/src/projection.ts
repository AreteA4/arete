import { hashFramedTuple, hashJcs } from "./canonical.js";
import { hashError } from "./error.js";
import { parseHashId } from "./hash.js";
import type {
  CompilerHash,
  IdlContentHash,
  IdlNormalizedHash,
  JsonValue,
  ProgramReleaseHash,
  ProgramSpecHash,
  SdkDefinitionHash,
  TupleField,
} from "./types.js";

export const COMPILER_SCHEMA_V1 = "arete.compiler/v1" as const;
export const SDK_DEFINITION_SCHEMA_V1 = "arete.sdk-definition/v1" as const;
export const SDK_DEFINITION_PROGRAM_SPEC_INPUT_KIND = "program-spec" as const;
export const OSS_DECODER_ENGINE_ID = "arete-oss-generated-decoder/v1" as const;
export const PROGRAM_RELEASE_SCHEMA_V1 = "arete.program-release/v1" as const;
export const HOSTED_MANAGED_RELEASE_PROFILE = "hosted-managed" as const;
export const OSS_GENERATED_RELEASE_PROFILE = "oss-generated" as const;

export interface HostedManagedProgramReleaseV1 {
  readonly schema: typeof PROGRAM_RELEASE_SCHEMA_V1;
  readonly releaseProfile: typeof HOSTED_MANAGED_RELEASE_PROFILE;
  readonly programId: string;
  readonly programSpecHash: ProgramSpecHash;
  readonly idlContentHash: IdlContentHash;
  readonly normalizedIdlHash: IdlNormalizedHash;
  readonly decoderAbiVersion: string;
  readonly decoderEngineId: string;
  readonly decoderBindingId: string;
}

export interface OssGeneratedProgramReleaseV1 {
  readonly schema: typeof PROGRAM_RELEASE_SCHEMA_V1;
  readonly releaseProfile: typeof OSS_GENERATED_RELEASE_PROFILE;
  readonly programId: string;
  readonly programSpecHash: ProgramSpecHash;
  readonly idlContentHash: IdlContentHash;
  readonly normalizedIdlHash: IdlNormalizedHash;
  readonly decoderEngineId: string;
}

export interface CompilerSourceV1 {
  readonly path: string;
  readonly bytes: Uint8Array;
}

export interface CompilerV1 {
  readonly schema: typeof COMPILER_SCHEMA_V1;
  readonly sources: readonly CompilerSourceV1[];
}

export interface SdkDefinitionV1 {
  readonly schema: typeof SDK_DEFINITION_SCHEMA_V1;
  readonly inputKind: typeof SDK_DEFINITION_PROGRAM_SPEC_INPUT_KIND;
  readonly inputHash: ProgramSpecHash;
  readonly compilerHash: CompilerHash;
}

export function createCompilerV1(
  sources: readonly CompilerSourceV1[],
): CompilerV1 {
  const sorted = sources
    .map((source) => ({ path: source.path, bytes: source.bytes.slice() }))
    .sort((left, right) => compareBytes(encoder.encode(left.path), encoder.encode(right.path)));
  validateCompilerV1({ schema: COMPILER_SCHEMA_V1, sources: sorted });
  return { schema: COMPILER_SCHEMA_V1, sources: sorted };
}

export function hashCompilerV1(projection: CompilerV1): CompilerHash {
  validateCompilerV1(projection);
  const fields: TupleField[] = [
    { label: "schema", value: encoder.encode(projection.schema) },
    ...projection.sources.map((source) => ({
      label: source.path,
      value: source.bytes,
    })),
  ];
  return hashFramedTuple("compiler", fields);
}

export function createSdkDefinitionV1(
  inputHash: ProgramSpecHash,
  compilerHash: CompilerHash,
): SdkDefinitionV1 {
  return {
    schema: SDK_DEFINITION_SCHEMA_V1,
    inputKind: SDK_DEFINITION_PROGRAM_SPEC_INPUT_KIND,
    inputHash,
    compilerHash,
  };
}

export function hashSdkDefinitionV1(
  projection: SdkDefinitionV1,
): SdkDefinitionHash {
  if (projection.schema !== SDK_DEFINITION_SCHEMA_V1) {
    return hashError("unknown-version", `unknown hash protocol version '${projection.schema}'`);
  }
  if (projection.inputKind !== SDK_DEFINITION_PROGRAM_SPEC_INPUT_KIND) {
    return hashError(
      "invalid-projection",
      `invalid SDK definition projection: inputKind must be '${SDK_DEFINITION_PROGRAM_SPEC_INPUT_KIND}', not '${projection.inputKind}'`,
    );
  }
  parseHashId(projection.inputHash, "program-spec");
  parseHashId(projection.compilerHash, "compiler");
  return hashJcs("sdk-definition", projection as unknown as JsonValue);
}

const encoder = new TextEncoder();

function validateCompilerV1(projection: CompilerV1): void {
  if (projection.schema !== COMPILER_SCHEMA_V1) {
    return hashError("unknown-version", `unknown hash protocol version '${projection.schema}'`);
  }
  if (projection.sources.length === 0) {
    return hashError("invalid-projection", "invalid compiler projection: sources must not be empty");
  }
  let previous: Uint8Array | undefined;
  for (const source of projection.sources) {
    if (source.path.length === 0 || source.path === "schema") {
      return hashError(
        "invalid-projection",
        `invalid compiler projection: invalid source path '${source.path}'`,
      );
    }
    const path = encoder.encode(source.path);
    if (previous !== undefined) {
      const order = compareBytes(previous, path);
      if (order > 0) {
        return hashError(
          "invalid-projection",
          "invalid compiler projection: sources must be sorted by raw UTF-8 path bytes",
        );
      }
      if (order === 0) {
        return hashError(
          "invalid-projection",
          `invalid compiler projection: duplicate source path '${source.path}'`,
        );
      }
    }
    previous = path;
  }
}

function compareBytes(left: Uint8Array, right: Uint8Array): number {
  const length = Math.min(left.length, right.length);
  for (let index = 0; index < length; index += 1) {
    const difference = left[index]! - right[index]!;
    if (difference !== 0) return difference;
  }
  return left.length - right.length;
}

export function projectWithoutArtifactHash(value: JsonValue): JsonValue {
  if (value === null || Array.isArray(value) || typeof value !== "object") {
    return hashError(
      "invalid-self-hash-projection",
      "self-hash projection must be a JSON object",
    );
  }
  const projection = { ...value };
  delete projection.artifactHash;
  return projection;
}

export function createOssGeneratedProgramReleaseV1(
  programId: string,
  programSpecHash: ProgramSpecHash,
  idlContentHash: IdlContentHash,
  normalizedIdlHash: IdlNormalizedHash,
  decoderEngineId: string = OSS_DECODER_ENGINE_ID,
): OssGeneratedProgramReleaseV1 {
  return {
    schema: PROGRAM_RELEASE_SCHEMA_V1,
    releaseProfile: OSS_GENERATED_RELEASE_PROFILE,
    programId,
    programSpecHash,
    idlContentHash,
    normalizedIdlHash,
    decoderEngineId,
  };
}

export function createHostedManagedProgramReleaseV1(
  programId: string,
  programSpecHash: ProgramSpecHash,
  idlContentHash: IdlContentHash,
  normalizedIdlHash: IdlNormalizedHash,
  decoderAbiVersion: string,
  decoderEngineId: string,
  decoderBindingId: string,
): HostedManagedProgramReleaseV1 {
  return {
    schema: PROGRAM_RELEASE_SCHEMA_V1,
    releaseProfile: HOSTED_MANAGED_RELEASE_PROFILE,
    programId,
    programSpecHash,
    idlContentHash,
    normalizedIdlHash,
    decoderAbiVersion,
    decoderEngineId,
    decoderBindingId,
  };
}

export function hashOssGeneratedProgramReleaseV1(
  projection: OssGeneratedProgramReleaseV1,
): ProgramReleaseHash {
  validateReleaseProjection(projection, OSS_GENERATED_RELEASE_PROFILE);
  return hashJcs("program-release", projection as unknown as JsonValue);
}

export function hashHostedManagedProgramReleaseV1(
  projection: HostedManagedProgramReleaseV1,
): ProgramReleaseHash {
  validateReleaseProjection(projection, HOSTED_MANAGED_RELEASE_PROFILE);
  return hashJcs("program-release", projection as unknown as JsonValue);
}

function validateReleaseProjection(
  projection: HostedManagedProgramReleaseV1 | OssGeneratedProgramReleaseV1,
  expectedProfile: string,
): void {
  if (projection.schema !== PROGRAM_RELEASE_SCHEMA_V1) {
    hashError(
      "unknown-version",
      `unknown hash protocol version '${projection.schema}'`,
    );
  }
  if (projection.releaseProfile !== expectedProfile) {
    hashError(
      "invalid-projection",
      `invalid program release projection: releaseProfile must be '${expectedProfile}', not '${projection.releaseProfile}'`,
    );
  }
  if (projection.programId.length === 0) {
    hashError(
      "invalid-projection",
      "invalid program release projection: programId must not be empty",
    );
  }
  if (projection.decoderEngineId.length === 0) {
    hashError(
      "invalid-projection",
      "invalid program release projection: decoderEngineId must not be empty",
    );
  }
  if (
    projection.releaseProfile === HOSTED_MANAGED_RELEASE_PROFILE &&
    "decoderAbiVersion" in projection &&
    projection.decoderAbiVersion.length === 0
  ) {
    hashError(
      "invalid-projection",
      "invalid program release projection: decoderAbiVersion must not be empty",
    );
  }
  if (
    projection.releaseProfile === HOSTED_MANAGED_RELEASE_PROFILE &&
    "decoderBindingId" in projection &&
    projection.decoderBindingId.length === 0
  ) {
    hashError(
      "invalid-projection",
      "invalid program release projection: decoderBindingId must not be empty",
    );
  }
  parseHashId(projection.programSpecHash, "program-spec");
  parseHashId(projection.idlContentHash, "idl-content");
  parseHashId(projection.normalizedIdlHash, "idl-normalized");
}
