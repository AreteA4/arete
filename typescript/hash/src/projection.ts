import { isCanonicalBase58_32 } from "./base58.js";
import { hashFramedTuple, hashJcs, parseJsonBytesStrict } from "./canonical.js";
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
  SdkOutputTreeHash,
  TupleField,
} from "./types.js";

export const COMPILER_SCHEMA_V1 = "arete.compiler/v1" as const;
export const SDK_DEFINITION_SCHEMA_V1 = "arete.sdk-definition/v1" as const;
export const SDK_DEFINITION_PROGRAM_SPEC_INPUT_KIND = "program-spec" as const;
export const OSS_DECODER_ENGINE_ID = "arete-oss-generated-decoder/v1" as const;
export const PROGRAM_RELEASE_SCHEMA_V1 = "arete.program-release/v1" as const;
export const PROGRAM_RELEASE_SCHEMA_V2 = "arete.program-release/v2" as const;
export const PROGRAM_RELEASE_SCHEMA_V3 = "arete.program-release/v3" as const;
export const HOSTED_MANAGED_RELEASE_PROFILE = "hosted-managed" as const;
export const HOSTED_PRIVATE_RELEASE_PROFILE = "hosted-private" as const;
export const HOSTED_PRIVATE_EXECUTABLE_POLICY = "observed" as const;
export const OSS_GENERATED_RELEASE_PROFILE = "oss-generated" as const;
export const SOLANA_EXECUTABLE_IDENTITY_SCHEMA_V1 =
  "arete.solana-executable-identity/v1" as const;
export const SOLANA_BPF_LOADER_V2_PROGRAM_ID =
  "BPFLoader2111111111111111111111111111111111" as const;
export const SOLANA_BPF_UPGRADEABLE_LOADER_PROGRAM_ID =
  "BPFLoaderUpgradeab1e11111111111111111111111" as const;
export const SOLANA_EXECUTABLE_PAYLOAD_SHA256_PREFIX = "sha256:" as const;

export type SolanaExecutablePayloadSha256 = `sha256:${string}`;

export type SolanaUpgradeAuthorityV1 =
  | { readonly kind: "none" }
  | { readonly kind: "address"; readonly address: string };

export type SolanaExecutableLoaderV1 =
  | {
      readonly kind: "bpf-loader-v2";
      readonly loaderProgramId: typeof SOLANA_BPF_LOADER_V2_PROGRAM_ID;
      readonly executablePayloadSha256: SolanaExecutablePayloadSha256;
    }
  | {
      readonly kind: "bpf-upgradeable-loader";
      readonly loaderProgramId: typeof SOLANA_BPF_UPGRADEABLE_LOADER_PROGRAM_ID;
      readonly programDataAddress: string;
      readonly deploymentSlot: string;
      readonly upgradeAuthority: SolanaUpgradeAuthorityV1;
      readonly executablePayloadSha256: SolanaExecutablePayloadSha256;
    };

export interface SolanaExecutableIdentityV1 {
  readonly schema: typeof SOLANA_EXECUTABLE_IDENTITY_SCHEMA_V1;
  readonly genesisHash: string;
  readonly loader: SolanaExecutableLoaderV1;
}

export interface HostedManagedProgramReleaseV2 {
  readonly schema: typeof PROGRAM_RELEASE_SCHEMA_V2;
  readonly releaseProfile: typeof HOSTED_MANAGED_RELEASE_PROFILE;
  readonly programId: string;
  readonly programSpecHash: ProgramSpecHash;
  readonly idlContentHash: IdlContentHash;
  readonly normalizedIdlHash: IdlNormalizedHash;
  readonly decoderAbiVersion: string;
  readonly decoderEngineId: string;
  readonly decoderBindingId: string;
  readonly executableIdentity: SolanaExecutableIdentityV1;
}

export interface HostedPrivateProgramReleaseV3 {
  readonly schema: typeof PROGRAM_RELEASE_SCHEMA_V3;
  readonly releaseProfile: typeof HOSTED_PRIVATE_RELEASE_PROFILE;
  readonly programId: string;
  readonly programSpecHash: ProgramSpecHash;
  readonly idlContentHash: IdlContentHash;
  readonly normalizedIdlHash: IdlNormalizedHash;
  readonly decoderAbiVersion: string;
  readonly decoderEngineId: string;
  readonly decoderBindingId: string;
  readonly executablePolicy: typeof HOSTED_PRIVATE_EXECUTABLE_POLICY;
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

export const SDK_DEFINITION_SCHEMA_V2 = "arete.sdk-definition/v2" as const;

export interface SdkDefinitionV2 {
  readonly schema: typeof SDK_DEFINITION_SCHEMA_V2;
  readonly inputKind: typeof SDK_DEFINITION_PROGRAM_SPEC_INPUT_KIND;
  readonly inputHash: ProgramSpecHash;
  readonly target: "typescript" | "rust" | "python";
  readonly runtimeContract: string;
  readonly outputTreeHash: SdkOutputTreeHash;
}

export function createSdkDefinitionV2(
  inputHash: ProgramSpecHash,
  target: SdkDefinitionV2["target"],
  runtimeContract: string,
  outputTreeHash: SdkOutputTreeHash,
): SdkDefinitionV2 {
  return {
    schema: SDK_DEFINITION_SCHEMA_V2,
    inputKind: SDK_DEFINITION_PROGRAM_SPEC_INPUT_KIND,
    inputHash,
    target,
    runtimeContract,
    outputTreeHash,
  };
}

export function hashSdkDefinitionV2(projection: SdkDefinitionV2): SdkDefinitionHash {
  if (projection.schema !== SDK_DEFINITION_SCHEMA_V2) {
    return hashError("unknown-version", `unknown hash protocol version '${projection.schema}'`);
  }
  const fields = ["schema", "inputKind", "inputHash", "target", "runtimeContract", "outputTreeHash"];
  if (
    Object.keys(projection).some((key) => !fields.includes(key))
    || projection.inputKind !== SDK_DEFINITION_PROGRAM_SPEC_INPUT_KIND
    || !["typescript", "rust", "python"].includes(projection.target)
    || typeof projection.runtimeContract !== "string"
    || !/^[\x21-\x7e]{1,128}$/.test(projection.runtimeContract)
  ) {
    return hashError("invalid-projection", "invalid SDK definition V2 projection");
  }
  parseHashId(projection.inputHash, "program-spec");
  parseHashId(projection.outputTreeHash, "sdk-output-tree");
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

export function createBpfLoaderV2ExecutableIdentityV1(
  genesisHash: string,
  executablePayloadSha256: string,
): SolanaExecutableIdentityV1 {
  return validateSolanaExecutableIdentityV1({
    schema: SOLANA_EXECUTABLE_IDENTITY_SCHEMA_V1,
    genesisHash,
    loader: {
      kind: "bpf-loader-v2",
      loaderProgramId: SOLANA_BPF_LOADER_V2_PROGRAM_ID,
      executablePayloadSha256,
    },
  });
}

export function createBpfUpgradeableLoaderExecutableIdentityV1(
  genesisHash: string,
  programDataAddress: string,
  deploymentSlot: bigint,
  upgradeAuthority: SolanaUpgradeAuthorityV1,
  executablePayloadSha256: string,
): SolanaExecutableIdentityV1 {
  if (deploymentSlot < 0n || deploymentSlot > U64_MAX) {
    return invalidProgramRelease(
      "deploymentSlot must be a canonical unsigned decimal u64 string",
    );
  }
  return validateSolanaExecutableIdentityV1({
    schema: SOLANA_EXECUTABLE_IDENTITY_SCHEMA_V1,
    genesisHash,
    loader: {
      kind: "bpf-upgradeable-loader",
      loaderProgramId: SOLANA_BPF_UPGRADEABLE_LOADER_PROGRAM_ID,
      programDataAddress,
      deploymentSlot: deploymentSlot.toString(),
      upgradeAuthority,
      executablePayloadSha256,
    },
  });
}

export function parseSolanaExecutableIdentityV1(
  bytes: Uint8Array,
): SolanaExecutableIdentityV1 {
  return validateSolanaExecutableIdentityV1(parseJsonBytesStrict(bytes));
}

export function validateSolanaExecutableIdentityV1(
  value: unknown,
): SolanaExecutableIdentityV1 {
  const identity = expectObject(value, "executableIdentity");
  expectKeys(identity, ["schema", "genesisHash", "loader"], "executableIdentity");
  if (identity.schema !== SOLANA_EXECUTABLE_IDENTITY_SCHEMA_V1) {
    return hashError(
      "unknown-version",
      `unknown Solana executable identity schema '${String(identity.schema)}'`,
    );
  }
  const genesisHash = expectBase58_32(identity.genesisHash, "genesisHash");
  const loader = validateSolanaExecutableLoaderV1(identity.loader);
  return {
    schema: SOLANA_EXECUTABLE_IDENTITY_SCHEMA_V1,
    genesisHash,
    loader,
  };
}

export function createHostedManagedProgramReleaseV2(
  programId: string,
  programSpecHash: ProgramSpecHash,
  idlContentHash: IdlContentHash,
  normalizedIdlHash: IdlNormalizedHash,
  decoderAbiVersion: string,
  decoderEngineId: string,
  decoderBindingId: string,
  executableIdentity: SolanaExecutableIdentityV1,
): HostedManagedProgramReleaseV2 {
  return validateHostedManagedProgramReleaseV2({
    schema: PROGRAM_RELEASE_SCHEMA_V2,
    releaseProfile: HOSTED_MANAGED_RELEASE_PROFILE,
    programId,
    programSpecHash,
    idlContentHash,
    normalizedIdlHash,
    decoderAbiVersion,
    decoderEngineId,
    decoderBindingId,
    executableIdentity,
  });
}

export function createHostedPrivateProgramReleaseV3(
  programId: string,
  programSpecHash: ProgramSpecHash,
  idlContentHash: IdlContentHash,
  normalizedIdlHash: IdlNormalizedHash,
  decoderAbiVersion: string,
  decoderEngineId: string,
  decoderBindingId: string,
): HostedPrivateProgramReleaseV3 {
  return validateHostedPrivateProgramReleaseV3({
    schema: PROGRAM_RELEASE_SCHEMA_V3,
    releaseProfile: HOSTED_PRIVATE_RELEASE_PROFILE,
    programId,
    programSpecHash,
    idlContentHash,
    normalizedIdlHash,
    decoderAbiVersion,
    decoderEngineId,
    decoderBindingId,
    executablePolicy: HOSTED_PRIVATE_EXECUTABLE_POLICY,
  });
}

export function hashOssGeneratedProgramReleaseV1(
  projection: OssGeneratedProgramReleaseV1,
): ProgramReleaseHash {
  validateReleaseProjection(projection, OSS_GENERATED_RELEASE_PROFILE);
  return hashJcs("program-release", projection as unknown as JsonValue);
}

export function parseHostedManagedProgramReleaseV2(
  bytes: Uint8Array,
): HostedManagedProgramReleaseV2 {
  return validateHostedManagedProgramReleaseV2(parseJsonBytesStrict(bytes));
}

export function hashHostedManagedProgramReleaseV2(
  projection: unknown,
): ProgramReleaseHash {
  const validated = validateHostedManagedProgramReleaseV2(projection);
  return hashJcs("program-release", validated as unknown as JsonValue);
}

export function validateHostedManagedProgramReleaseV2(
  value: unknown,
): HostedManagedProgramReleaseV2 {
  const release = expectObject(value, "program release");
  expectKeys(
    release,
    [
      "schema",
      "releaseProfile",
      "programId",
      "programSpecHash",
      "idlContentHash",
      "normalizedIdlHash",
      "decoderAbiVersion",
      "decoderEngineId",
      "decoderBindingId",
      "executableIdentity",
    ],
    "program release",
  );
  if (release.schema !== PROGRAM_RELEASE_SCHEMA_V2) {
    return hashError(
      "unknown-version",
      `unknown program release schema '${String(release.schema)}'`,
    );
  }
  if (release.releaseProfile !== HOSTED_MANAGED_RELEASE_PROFILE) {
    return invalidProgramRelease(
      `releaseProfile must be '${HOSTED_MANAGED_RELEASE_PROFILE}', not '${String(release.releaseProfile)}'`,
    );
  }

  const programId = expectBase58_32(release.programId, "programId");
  const programSpecHash = expectTypedHash(
    release.programSpecHash,
    "program-spec",
    "programSpecHash",
  ) as ProgramSpecHash;
  const idlContentHash = expectTypedHash(
    release.idlContentHash,
    "idl-content",
    "idlContentHash",
  ) as IdlContentHash;
  const normalizedIdlHash = expectTypedHash(
    release.normalizedIdlHash,
    "idl-normalized",
    "normalizedIdlHash",
  ) as IdlNormalizedHash;
  const decoderAbiVersion = expectIdentifier(
    release.decoderAbiVersion,
    "decoderAbiVersion",
    64,
  );
  const decoderEngineId = expectIdentifier(
    release.decoderEngineId,
    "decoderEngineId",
    128,
  );
  const decoderBindingId = expectIdentifier(
    release.decoderBindingId,
    "decoderBindingId",
    128,
  );
  const executableIdentity = validateSolanaExecutableIdentityV1(
    release.executableIdentity,
  );

  return {
    schema: PROGRAM_RELEASE_SCHEMA_V2,
    releaseProfile: HOSTED_MANAGED_RELEASE_PROFILE,
    programId,
    programSpecHash,
    idlContentHash,
    normalizedIdlHash,
    decoderAbiVersion,
    decoderEngineId,
    decoderBindingId,
    executableIdentity,
  };
}

export function parseHostedPrivateProgramReleaseV3(
  bytes: Uint8Array,
): HostedPrivateProgramReleaseV3 {
  return validateHostedPrivateProgramReleaseV3(parseJsonBytesStrict(bytes));
}

export function hashHostedPrivateProgramReleaseV3(
  projection: unknown,
): ProgramReleaseHash {
  const validated = validateHostedPrivateProgramReleaseV3(projection);
  return hashJcs("program-release", validated as unknown as JsonValue);
}

export function validateHostedPrivateProgramReleaseV3(
  value: unknown,
): HostedPrivateProgramReleaseV3 {
  const release = expectObject(value, "program release");
  expectKeys(
    release,
    [
      "schema",
      "releaseProfile",
      "programId",
      "programSpecHash",
      "idlContentHash",
      "normalizedIdlHash",
      "decoderAbiVersion",
      "decoderEngineId",
      "decoderBindingId",
      "executablePolicy",
    ],
    "program release",
  );
  if (release.schema !== PROGRAM_RELEASE_SCHEMA_V3) {
    return hashError(
      "unknown-version",
      `unknown program release schema '${String(release.schema)}'`,
    );
  }
  if (release.releaseProfile !== HOSTED_PRIVATE_RELEASE_PROFILE) {
    return invalidProgramRelease(
      `releaseProfile must be '${HOSTED_PRIVATE_RELEASE_PROFILE}', not '${String(release.releaseProfile)}'`,
    );
  }
  if (release.executablePolicy !== HOSTED_PRIVATE_EXECUTABLE_POLICY) {
    return invalidProgramRelease(
      `executablePolicy must be '${HOSTED_PRIVATE_EXECUTABLE_POLICY}', not '${String(release.executablePolicy)}'`,
    );
  }
  const decoderBindingId = expectIdentifier(
    release.decoderBindingId,
    "decoderBindingId",
    128,
  );
  if (!/^dec_[A-Za-z0-9_-]{32}$/.test(decoderBindingId)) {
    return invalidProgramRelease(
      "decoderBindingId must begin with 'dec_' and contain exactly 32 URL-safe characters",
    );
  }
  return {
    schema: PROGRAM_RELEASE_SCHEMA_V3,
    releaseProfile: HOSTED_PRIVATE_RELEASE_PROFILE,
    programId: expectBase58_32(release.programId, "programId"),
    programSpecHash: expectTypedHash(
      release.programSpecHash,
      "program-spec",
      "programSpecHash",
    ) as ProgramSpecHash,
    idlContentHash: expectTypedHash(
      release.idlContentHash,
      "idl-content",
      "idlContentHash",
    ) as IdlContentHash,
    normalizedIdlHash: expectTypedHash(
      release.normalizedIdlHash,
      "idl-normalized",
      "normalizedIdlHash",
    ) as IdlNormalizedHash,
    decoderAbiVersion: expectIdentifier(
      release.decoderAbiVersion,
      "decoderAbiVersion",
      64,
    ),
    decoderEngineId: expectIdentifier(
      release.decoderEngineId,
      "decoderEngineId",
      128,
    ),
    decoderBindingId,
    executablePolicy: HOSTED_PRIVATE_EXECUTABLE_POLICY,
  };
}

function validateSolanaExecutableLoaderV1(
  value: unknown,
): SolanaExecutableLoaderV1 {
  const loader = expectObject(value, "executableIdentity.loader");
  if (loader.kind === "bpf-loader-v2") {
    expectKeys(
      loader,
      ["kind", "loaderProgramId", "executablePayloadSha256"],
      "executableIdentity.loader",
    );
    expectExactString(
      loader.loaderProgramId,
      SOLANA_BPF_LOADER_V2_PROGRAM_ID,
      "loaderProgramId for 'bpf-loader-v2'",
    );
    return {
      kind: "bpf-loader-v2",
      loaderProgramId: SOLANA_BPF_LOADER_V2_PROGRAM_ID,
      executablePayloadSha256: expectSha256Digest(loader.executablePayloadSha256),
    };
  }
  if (loader.kind === "bpf-upgradeable-loader") {
    expectKeys(
      loader,
      [
        "kind",
        "loaderProgramId",
        "programDataAddress",
        "deploymentSlot",
        "upgradeAuthority",
        "executablePayloadSha256",
      ],
      "executableIdentity.loader",
    );
    expectExactString(
      loader.loaderProgramId,
      SOLANA_BPF_UPGRADEABLE_LOADER_PROGRAM_ID,
      "loaderProgramId for 'bpf-upgradeable-loader'",
    );
    return {
      kind: "bpf-upgradeable-loader",
      loaderProgramId: SOLANA_BPF_UPGRADEABLE_LOADER_PROGRAM_ID,
      programDataAddress: expectBase58_32(
        loader.programDataAddress,
        "programDataAddress",
      ),
      deploymentSlot: expectDeploymentSlot(loader.deploymentSlot),
      upgradeAuthority: validateUpgradeAuthorityV1(loader.upgradeAuthority),
      executablePayloadSha256: expectSha256Digest(loader.executablePayloadSha256),
    };
  }
  return invalidProgramRelease(
    "executableIdentity.loader.kind must be 'bpf-loader-v2' or 'bpf-upgradeable-loader'",
  );
}

function validateUpgradeAuthorityV1(value: unknown): SolanaUpgradeAuthorityV1 {
  const authority = expectObject(value, "upgradeAuthority");
  if (authority.kind === "none") {
    expectKeys(authority, ["kind"], "upgradeAuthority");
    return { kind: "none" };
  }
  if (authority.kind === "address") {
    expectKeys(authority, ["kind", "address"], "upgradeAuthority");
    return {
      kind: "address",
      address: expectBase58_32(authority.address, "upgradeAuthority.address"),
    };
  }
  return invalidProgramRelease(
    "upgradeAuthority.kind must be 'none' or 'address'",
  );
}

function expectObject(value: unknown, field: string): Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return invalidProgramRelease(`${field} must be an object`);
  }
  return value as Record<string, unknown>;
}

function expectKeys(
  value: Record<string, unknown>,
  keys: readonly string[],
  field: string,
): void {
  const allowed = new Set(keys);
  for (const key of Object.keys(value)) {
    if (!allowed.has(key)) {
      invalidProgramRelease(`${field} contains unknown field '${key}'`);
    }
  }
  for (const key of keys) {
    if (!Object.hasOwn(value, key)) {
      invalidProgramRelease(`${field} is missing '${key}'`);
    }
  }
}

function expectExactString(
  value: unknown,
  expected: string,
  field: string,
): void {
  if (value !== expected) {
    invalidProgramRelease(`${field} must be '${expected}', not '${String(value)}'`);
  }
}

function expectTypedHash(value: unknown, kind: Parameters<typeof parseHashId>[1], field: string): string {
  if (typeof value !== "string") {
    return invalidProgramRelease(`${field} must be a ${kind} typed hash`);
  }
  try {
    return parseHashId(value, kind).id;
  } catch {
    return invalidProgramRelease(`${field} must be a ${kind} typed hash`);
  }
}

function expectIdentifier(value: unknown, field: string, maxLength: number): string {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.trim() !== value ||
    encoder.encode(value).length > maxLength
  ) {
    return invalidProgramRelease(
      `${field} must be a nonempty, trimmed string of at most ${maxLength} bytes`,
    );
  }
  return value;
}

function expectDeploymentSlot(value: unknown): string {
  if (
    typeof value !== "string" ||
    !/^(?:0|[1-9][0-9]*)$/.test(value) ||
    BigInt(value) > U64_MAX
  ) {
    return invalidProgramRelease(
      "deploymentSlot must be a canonical unsigned decimal u64 string",
    );
  }
  return value;
}

function expectSha256Digest(value: unknown): SolanaExecutablePayloadSha256 {
  if (typeof value !== "string" || !/^sha256:[0-9a-f]{64}$/.test(value)) {
    return invalidProgramRelease(
      "executablePayloadSha256 must use the sha256:<lowercase-hex> format",
    );
  }
  return value as SolanaExecutablePayloadSha256;
}

function expectBase58_32(value: unknown, field: string): string {
  if (!isCanonicalBase58_32(value)) {
    return invalidProgramRelease(`${field} must be a canonical 32-byte base58 value`);
  }
  return value;
}

function invalidProgramRelease<T>(reason: string): T {
  return hashError(
    "invalid-projection",
    `invalid program release projection: ${reason}`,
  );
}

const U64_MAX = 18_446_744_073_709_551_615n;

function validateReleaseProjection(
  projection: OssGeneratedProgramReleaseV1,
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
  parseHashId(projection.programSpecHash, "program-spec");
  parseHashId(projection.idlContentHash, "idl-content");
  parseHashId(projection.normalizedIdlHash, "idl-normalized");
}
