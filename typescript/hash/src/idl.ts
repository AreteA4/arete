import { sha256 } from "@noble/hashes/sha256";

import { hashJcs, hashRawBytes, parseJsonBytesStrict } from "./canonical.js";
import { hashError } from "./error.js";
import {
  createOssGeneratedProgramReleaseV1,
  hashOssGeneratedProgramReleaseV1,
  type OssGeneratedProgramReleaseV1,
} from "./projection.js";
import type {
  IdlContentHash,
  IdlNormalizedHash,
  IdlPortableHash,
  IdlSourceHash,
  JsonValue,
  ProgramReleaseHash,
  ProgramSpecHash,
} from "./types.js";

export const IDL_NORMALIZATION_VERSION = 1 as const;
export const PROGRAM_SPEC_SCHEMA_V1 = "arete.program-spec/v1" as const;

type JsonObject = { [key: string]: JsonValue };
type UnknownObject = Record<string, unknown>;

export interface IdlProjectionHashesV1 {
  readonly source: IdlSourceHash;
  readonly content: IdlContentHash;
  readonly portable: IdlPortableHash;
}

export interface IdlProjectionsV1 {
  readonly sourceBytes: Uint8Array;
  readonly contentProjection: JsonObject;
  readonly portableProjection: JsonObject;
  readonly programId: string;
  readonly hashes: IdlProjectionHashesV1;
}

export interface IdlHashesV1 extends IdlProjectionHashesV1 {
  readonly normalized: IdlNormalizedHash;
  readonly programSpec: ProgramSpecHash;
  readonly ossRelease: ProgramReleaseHash;
}

export interface IdlSnapshotV1 extends JsonObject {
  normalizationVersion: typeof IDL_NORMALIZATION_VERSION;
  name: string;
  program_id: string;
  version: string;
  accounts: JsonValue[];
  instructions: JsonValue[];
  types: JsonValue[];
  events: JsonValue[];
  errors: JsonValue[];
  discriminant_size: number;
}

export type PdaSeedV1 =
  | { type: "literal"; value: string }
  | { type: "bytes"; value: number[] }
  | { type: "argRef"; arg_name: string; arg_type?: string }
  | { type: "accountRef"; account_name: string };

export interface PdaDefinitionV1 {
  name: string;
  seeds: PdaSeedV1[];
  program_id?: string;
}

export type AccountResolutionV1 =
  | { category: "signer" }
  | { category: "known"; address: string }
  | { category: "pdaRef"; pda_name: string }
  | { category: "pdaInline"; seeds: PdaSeedV1[]; program_id?: string }
  | { category: "userProvided" };

export interface InstructionAccountV1 {
  name: string;
  is_signer: boolean;
  is_writable: boolean;
  resolution: AccountResolutionV1;
  is_optional: boolean;
  docs?: string[];
}

export interface InstructionArgumentV1 {
  name: string;
  type: string;
  docs?: string[];
  amount_hint?: JsonValue;
}

export interface InstructionDefinitionV1 {
  name: string;
  discriminator: number[];
  discriminator_size: number;
  accounts: InstructionAccountV1[];
  args: InstructionArgumentV1[];
  errors?: JsonValue[];
  program_id?: string;
  docs?: string[];
}

export interface ProgramSpecV1 {
  schema: typeof PROGRAM_SPEC_SCHEMA_V1;
  programId: string;
  idlContentHash: IdlContentHash;
  portableIdlHash: IdlPortableHash;
  normalizedIdlHash: IdlNormalizedHash;
  idlSnapshot: IdlSnapshotV1;
  pdas: { [name: string]: PdaDefinitionV1 };
  instructions: InstructionDefinitionV1[];
}

export interface CanonicalIdlDocumentV1 extends IdlProjectionsV1 {
  readonly normalizedSnapshot: IdlSnapshotV1;
  readonly programSpec: ProgramSpecV1;
  readonly ossRelease: OssGeneratedProgramReleaseV1;
  readonly hashes: IdlHashesV1;
}

interface ParsedIdl {
  version?: string;
  name?: string;
  address: string;
  instructions: ParsedInstruction[];
  accounts: ParsedAccount[];
  types: ParsedTypeDef[];
  events: ParsedEvent[];
  errors: ParsedError[];
  pdas: ParsedNamedPda[];
  metadata?: { name?: string; version?: string; address?: string };
}

interface ParsedInstruction {
  name: string;
  discriminator: number[];
  discriminant: { type: string; value: number } | null;
  docs: string[];
  accounts: ParsedInstructionAccount[];
  args: ParsedField[];
}

interface ParsedInstructionAccount {
  name: string;
  isMut: boolean;
  isSigner: boolean;
  address?: string;
  optional: boolean;
  docs: string[];
  pda?: ParsedPda;
  accounts: ParsedInstructionAccount[];
}

interface ParsedPda {
  name?: string;
  seeds: ParsedPdaSeed[];
  program?: UnknownObject;
}

interface ParsedNamedPda extends ParsedPda {
  name: string;
}

type ParsedPdaSeed =
  | { kind: "const"; value: number[] }
  | { kind: "account"; path: string }
  | { kind: "arg"; path: string; argType?: string };

interface ParsedField {
  name: string;
  type: JsonValue;
  amountHint?: JsonValue;
}

interface ParsedAccount {
  name: string;
  discriminator: number[];
  docs: string[];
  typeDef?: ParsedTypeDefKind;
}

interface ParsedTypeDef {
  name: string;
  docs: string[];
  serialization?: string;
  typeDef: ParsedTypeDefKind;
}

type ParsedTypeDefKind =
  | { kind: string; fields: ParsedField[]; tuple: false }
  | { kind: string; fields: JsonValue[]; tuple: true }
  | { kind: string; variants: ParsedEnumVariant[] };

interface ParsedEnumVariant {
  name: string;
  fields: (ParsedField | JsonValue)[];
}

interface ParsedEvent {
  name: string;
  discriminator: number[];
  docs: string[];
  fields: ParsedField[];
}

interface ParsedError {
  code: number;
  name: string;
  msg?: string;
}

export function projectIdlV1(
  sourceBytes: Uint8Array,
  explicitProgramId?: string | null,
): IdlProjectionsV1 {
  const parsed = parseJsonBytesStrict(sourceBytes);
  if (!isObject(parsed)) return hashError("invalid-idl", "IDL root must be an object");
  const sourceProgramIds = collectProgramIds(parsed);
  const sourceHasProgramId = sourceProgramIds.length > 0;
  const programId = resolveProgramId(sourceProgramIds, explicitProgramId);
  const contentProjection = cloneJson(parsed) as JsonObject;
  if (!sourceHasProgramId) contentProjection.address = programId;
  const portableProjection = portableIdlProjection(contentProjection);
  return {
    sourceBytes: sourceBytes.slice(),
    contentProjection,
    portableProjection,
    programId,
    hashes: {
      source: hashRawBytes("idl-source", sourceBytes),
      content: hashJcs("idl-content", contentProjection),
      portable: hashJcs("idl-portable", portableProjection),
    },
  };
}

export function portableIdlProjection(source: JsonValue): JsonObject {
  if (!isObject(source)) return hashError("invalid-idl", "IDL root must be an object");
  const portable = cloneJson(source) as JsonObject;
  delete portable.address;
  delete portable.program_id;
  if (isObject(portable.metadata)) delete portable.metadata.address;
  if (isObject(portable.program)) delete portable.program.publicKey;
  return portable;
}

export function parseIdlV1(
  sourceBytes: Uint8Array,
  explicitProgramId?: string | null,
): CanonicalIdlDocumentV1 {
  const projections = projectIdlV1(sourceBytes, explicitProgramId);
  const idl = parseStandardIdl(projections.contentProjection, projections.programId);
  const normalizedSnapshot = normalizeIdlSnapshotV1(idl);
  const normalized = hashJcs("idl-normalized", normalizedSnapshot);
  const programSpec = createProgramSpecV1(idl, normalizedSnapshot, {
    content: projections.hashes.content,
    portable: projections.hashes.portable,
    normalized,
  });
  const programSpecHash = hashProgramSpecV1(programSpec);
  const ossRelease = createOssGeneratedProgramReleaseV1(
    projections.programId,
    programSpecHash,
    projections.hashes.content,
    normalized,
  );
  const ossReleaseHash = hashOssGeneratedProgramReleaseV1(ossRelease);
  return {
    ...projections,
    normalizedSnapshot,
    programSpec,
    ossRelease,
    hashes: {
      ...projections.hashes,
      normalized,
      programSpec: programSpecHash,
      ossRelease: ossReleaseHash,
    },
  };
}

export function buildProgramSpecV1FromBytes(
  sourceBytes: Uint8Array,
  explicitProgramId?: string | null,
): ProgramSpecV1 {
  return parseIdlV1(sourceBytes, explicitProgramId).programSpec;
}

export function hashProgramSpecV1(programSpec: ProgramSpecV1): ProgramSpecHash {
  validateProgramSpecV1(programSpec);
  return hashJcs("program-spec", programSpec as unknown as JsonValue);
}

export function validateProgramSpecV1(programSpec: ProgramSpecV1): void {
  if (programSpec.schema !== PROGRAM_SPEC_SCHEMA_V1) {
    hashError("unknown-version", `unknown hash protocol version '${programSpec.schema}'`);
  }
  if (programSpec.idlSnapshot.normalizationVersion !== IDL_NORMALIZATION_VERSION) {
    hashError(
      "unknown-version",
      `unknown IDL normalization version ${programSpec.idlSnapshot.normalizationVersion}`,
    );
  }
  if (programSpec.programId.length === 0) hashError("missing-program-id", "program ID is missing");
  if (programSpec.idlSnapshot.program_id !== programSpec.programId) {
    hashError(
      "invalid-projection",
      "invalid program spec projection: programId must match idlSnapshot.program_id",
    );
  }
}

function collectProgramIds(source: JsonObject): [string, string][] {
  const values: [string, string][] = [];
  collectProgramId(values, "address", source.address);
  collectProgramId(values, "program_id", source.program_id);
  collectNestedProgramId(values, "metadata.address", source.metadata, "address");
  collectNestedProgramId(values, "program.publicKey", source.program, "publicKey");
  return values;
}

function collectNestedProgramId(
  values: [string, string][],
  location: string,
  parent: JsonValue | undefined,
  key: string,
): void {
  if (parent === undefined || parent === null) return;
  if (!isObject(parent)) {
    hashError(
      "invalid-program-id-location",
      `program ID at '${location}' must be a string or null`,
    );
  }
  collectProgramId(values, location, parent[key]);
}

function collectProgramId(
  values: [string, string][],
  location: string,
  value: JsonValue | undefined,
): void {
  if (value === undefined || value === null || value === "") return;
  if (typeof value !== "string") {
    hashError(
      "invalid-program-id-location",
      `program ID at '${location}' must be a string or null`,
    );
  }
  values.push([location, value]);
}

function resolveProgramId(
  sourceValues: [string, string][],
  explicitProgramId?: string | null,
): string {
  const values = [...sourceValues];
  if (explicitProgramId !== undefined && explicitProgramId !== null) {
    if (explicitProgramId.length === 0) {
      return hashError("missing-program-id", "program ID is missing");
    }
    values.push(["explicit", explicitProgramId]);
  }
  if (values.length === 0) {
    return hashError(
      "missing-program-id",
      "program ID is missing from the IDL and no explicit program ID was supplied",
    );
  }
  if (new Set(values.map(([, value]) => value)).size !== 1) {
    return hashError(
      "conflicting-program-ids",
      `conflicting program IDs: ${values.map(([location, value]) => `${location}=${value}`).join(", ")}`,
    );
  }
  return values[0]?.[1] ?? hashError("missing-program-id", "program ID is missing");
}

function parseStandardIdl(value: JsonObject, programId: string): ParsedIdl {
  if (value.kind === "rootNode") {
    return hashError("invalid-idl", "Codama rootNode ProgramSpec projection is not supported");
  }
  const instructions = requiredArray(value.instructions, "instructions").map(parseInstruction);
  return {
    version: optionalString(value.version, "version"),
    name: optionalString(value.name, "name"),
    address: programId,
    instructions,
    accounts: optionalArray(value.accounts, "accounts").map(parseAccount),
    types: optionalArray(value.types, "types").map(parseTypeDef),
    events: optionalArray(value.events, "events").map(parseEvent),
    errors: optionalArray(value.errors, "errors").map(parseError),
    pdas: optionalArray(value.pdas, "pdas").map(parseNamedPda),
    metadata: value.metadata == null ? undefined : parseMetadata(value.metadata),
  };
}

function parseInstruction(value: unknown, index: number): ParsedInstruction {
  const item = requiredObject(value, `instructions[${index}]`);
  return {
    name: requiredString(item.name, `instructions[${index}].name`),
    discriminator: byteArray(item.discriminator, `instructions[${index}].discriminator`),
    discriminant: item.discriminant == null
      ? null
      : parseDiscriminant(item.discriminant, `instructions[${index}].discriminant`),
    docs: stringArray(item.docs, `instructions[${index}].docs`),
    accounts: requiredUnknownArray(item.accounts, `instructions[${index}].accounts`).map(
      parseInstructionAccount,
    ),
    args: requiredUnknownArray(item.args, `instructions[${index}].args`).map(parseField),
  };
}

function parseInstructionAccount(value: unknown, index: number): ParsedInstructionAccount {
  const item = requiredObject(value, `instruction account[${index}]`);
  return {
    name: requiredString(item.name, `instruction account[${index}].name`),
    isMut: optionalBoolean(item.isMut ?? item.writable, `instruction account[${index}].isMut`),
    isSigner: optionalBoolean(
      item.isSigner ?? item.signer,
      `instruction account[${index}].isSigner`,
    ),
    address: optionalString(item.address, `instruction account[${index}].address`),
    optional: optionalBoolean(item.optional, `instruction account[${index}].optional`),
    docs: stringArray(item.docs, `instruction account[${index}].docs`),
    pda: item.pda == null ? undefined : parsePda(item.pda, `instruction account[${index}].pda`),
    accounts: optionalUnknownArray(item.accounts, `instruction account[${index}].accounts`).map(
      parseInstructionAccount,
    ),
  };
}

function parseAccount(value: unknown, index: number): ParsedAccount {
  const item = requiredObject(value, `accounts[${index}]`);
  return {
    name: requiredString(item.name, `accounts[${index}].name`),
    discriminator: byteArray(item.discriminator, `accounts[${index}].discriminator`),
    docs: stringArray(item.docs, `accounts[${index}].docs`),
    typeDef: item.type == null ? undefined : parseTypeDefKind(item.type, `accounts[${index}].type`),
  };
}

function parseTypeDef(value: unknown, index: number): ParsedTypeDef {
  const item = requiredObject(value, `types[${index}]`);
  return {
    name: requiredString(item.name, `types[${index}].name`),
    docs: stringArray(item.docs, `types[${index}].docs`),
    serialization: optionalString(item.serialization, `types[${index}].serialization`),
    typeDef: parseTypeDefKind(item.type, `types[${index}].type`),
  };
}

function parseTypeDefKind(value: unknown, location: string): ParsedTypeDefKind {
  const item = requiredObject(value, location);
  const kind = requiredString(item.kind, `${location}.kind`);
  if (Array.isArray(item.variants)) {
    return {
      kind,
      variants: item.variants.map((variant, index) => {
        const parsed = requiredObject(variant, `${location}.variants[${index}]`);
        return {
          name: requiredString(parsed.name, `${location}.variants[${index}].name`),
          fields: optionalUnknownArray(parsed.fields, `${location}.variants[${index}].fields`).map(
            (field, fieldIndex) =>
              isObjectLike(field) && Object.hasOwn(field, "name")
                ? parseField(field, fieldIndex)
                : normalizeIdlType(field, `${location}.variants[${index}].fields[${fieldIndex}]`),
          ),
        };
      }),
    };
  }
  const fields = requiredUnknownArray(item.fields, `${location}.fields`);
  const tuple = fields.some((field) => !isObjectLike(field) || !Object.hasOwn(field, "name"));
  return tuple
    ? { kind, fields: fields.map((field, index) => normalizeIdlType(field, `${location}.fields[${index}]`)), tuple: true }
    : { kind, fields: fields.map(parseField), tuple: false };
}

function parseEvent(value: unknown, index: number): ParsedEvent {
  const item = requiredObject(value, `events[${index}]`);
  return {
    name: requiredString(item.name, `events[${index}].name`),
    discriminator: byteArray(item.discriminator, `events[${index}].discriminator`),
    docs: stringArray(item.docs, `events[${index}].docs`),
    fields: optionalUnknownArray(item.fields, `events[${index}].fields`).map(parseField),
  };
}

function parseError(value: unknown, index: number): ParsedError {
  const item = requiredObject(value, `errors[${index}]`);
  const code = item.code;
  if (typeof code !== "number" || !Number.isInteger(code) || code < 0 || code > 0xffffffff) {
    return invalidIdl(`errors[${index}].code must be a u32`);
  }
  return {
    code,
    name: requiredString(item.name, `errors[${index}].name`),
    msg: optionalString(item.msg, `errors[${index}].msg`),
  };
}

function parseField(value: unknown, index: number): ParsedField {
  const item = requiredObject(value, `field[${index}]`);
  return {
    name: requiredString(item.name, `field[${index}].name`),
    type: normalizeIdlType(item.type, `field[${index}].type`),
    amountHint: item.amountHint === undefined
      ? undefined
      : cloneJson(item.amountHint as JsonValue),
  };
}

function normalizeIdlType(value: unknown, location: string): JsonValue {
  if (typeof value === "string") return value;
  const item = requiredObject(value, location);
  if (Object.hasOwn(item, "option")) return { option: normalizeIdlType(item.option, `${location}.option`) };
  if (Object.hasOwn(item, "vec")) return { vec: normalizeIdlType(item.vec, `${location}.vec`) };
  const map = item.hashMap ?? item.bTreeMap;
  if (map !== undefined) {
    const values = requiredUnknownArray(map, `${location}.hashMap`);
    if (values.length !== 2) return invalidIdl(`${location}.hashMap must contain two types`);
    return { hashMap: values.map((child, index) => normalizeIdlType(child, `${location}.hashMap[${index}]`)) };
  }
  if (Object.hasOwn(item, "array")) {
    return {
      array: requiredUnknownArray(item.array, `${location}.array`).map((child, index) =>
        typeof child === "number" ? child : normalizeIdlType(child, `${location}.array[${index}]`),
      ),
    };
  }
  if (Object.hasOwn(item, "defined")) {
    const defined = item.defined;
    if (typeof defined === "string") return { defined };
    const named = requiredObject(defined, `${location}.defined`);
    return { defined: { name: requiredString(named.name, `${location}.defined.name`) } };
  }
  return invalidIdl(`${location} is not a supported IDL type`);
}

function parseNamedPda(value: unknown, index: number): ParsedNamedPda {
  const pda = parsePda(value, `pdas[${index}]`);
  return { ...pda, name: requiredString(requiredObject(value, `pdas[${index}]`).name, `pdas[${index}].name`) };
}

function parsePda(value: unknown, location: string): ParsedPda {
  const item = requiredObject(value, location);
  return {
    name: optionalString(item.name, `${location}.name`),
    seeds: requiredUnknownArray(item.seeds, `${location}.seeds`).map((seed, index) =>
      parsePdaSeed(seed, `${location}.seeds[${index}]`),
    ),
    program: item.program == null ? undefined : requiredObject(item.program, `${location}.program`),
  };
}

function parsePdaSeed(value: unknown, location: string): ParsedPdaSeed {
  const item = requiredObject(value, location);
  const kind = requiredString(item.kind, `${location}.kind`);
  if (kind === "const") return { kind, value: byteArray(item.value, `${location}.value`, true) };
  if (kind === "account") return { kind, path: requiredString(item.path, `${location}.path`) };
  if (kind === "arg") {
    return {
      kind,
      path: requiredString(item.path, `${location}.path`),
      argType: optionalString(item.type, `${location}.type`),
    };
  }
  return invalidIdl(`${location}.kind '${kind}' is unsupported`);
}

function parseDiscriminant(value: unknown, location: string): { type: string; value: number } {
  const item = requiredObject(value, location);
  const discriminantValue = item.value;
  if (
    typeof discriminantValue !== "number" ||
    !Number.isSafeInteger(discriminantValue) ||
    discriminantValue < 0
  ) {
    return invalidIdl(`${location}.value must be a non-negative integer`);
  }
  return { type: requiredString(item.type, `${location}.type`), value: discriminantValue };
}

function parseMetadata(value: unknown): ParsedIdl["metadata"] {
  const item = requiredObject(value, "metadata");
  return {
    name: optionalString(item.name, "metadata.name"),
    version: optionalString(item.version, "metadata.version"),
    address: optionalString(item.address, "metadata.address"),
  };
}

function normalizeIdlSnapshotV1(idl: ParsedIdl): IdlSnapshotV1 {
  const types = idl.types.map(snapshotTypeDef);
  for (const account of idl.accounts) {
    if (types.some((type) => type.name === account.name) || account.typeDef === undefined) continue;
    types.push({
      name: account.name,
      docs: account.docs,
      type: snapshotTypeDefKind(account.typeDef),
    });
  }
  const usesSteel = idl.instructions.some(
    (instruction) => instruction.discriminant !== null && instruction.discriminator.length === 0,
  );
  return {
    normalizationVersion: IDL_NORMALIZATION_VERSION,
    name: idl.name ?? idl.metadata?.name ?? "unknown",
    program_id: idl.address,
    version: idl.version ?? idl.metadata?.version ?? "0.1.0",
    accounts: idl.accounts.map((account) => {
      const matchingType = idl.types.find((type) => type.name === account.name);
      const output: JsonObject = {
        name: account.name,
        discriminator: discriminator(account.discriminator, `account:${account.name}`),
        docs: account.docs,
        serialization: matchingType?.serialization ?? null,
        fields:
          account.typeDef && "fields" in account.typeDef && !account.typeDef.tuple
            ? account.typeDef.fields.map(snapshotField)
            : [],
      };
      return output;
    }),
    instructions: idl.instructions.map((instruction) => ({
      name: instruction.name,
      discriminator: instructionDiscriminator(instruction),
      discriminant: instruction.discriminant,
      docs: instruction.docs,
      accounts: flattenAccounts(instruction.accounts).map((account) => ({
        name: account.name,
        writable: account.isMut,
        signer: account.isSigner,
        optional: account.optional,
        address: account.address ?? null,
        docs: account.docs,
      })),
      args: instruction.args.map(snapshotField),
    })),
    types,
    events: idl.events.map((event) => ({
      name: event.name,
      discriminator: discriminator(event.discriminator, `event:${event.name}`),
      docs: event.docs,
      fields: event.fields.map(snapshotField),
    })),
    errors: idl.errors.map((error) => ({
      code: error.code,
      name: error.name,
      ...(error.msg === undefined ? {} : { msg: error.msg }),
    })),
    discriminant_size: usesSteel ? 1 : 8,
  };
}

function snapshotTypeDef(type: ParsedTypeDef): JsonObject {
  return {
    name: type.name,
    docs: type.docs,
    ...(type.serialization === undefined ? {} : { serialization: type.serialization }),
    type: snapshotTypeDefKind(type.typeDef),
  };
}

function snapshotTypeDefKind(type: ParsedTypeDefKind): JsonObject {
  if ("variants" in type) {
    return {
      kind: type.kind,
      variants: type.variants.map((variant) => ({
        name: variant.name,
        ...(variant.fields.length === 0
          ? {}
          : {
              fields: variant.fields.map((field) =>
                isParsedField(field) ? snapshotField(field) : cloneJson(field),
              ),
            }),
      })),
    };
  }
  return {
    kind: type.kind,
    fields: type.tuple ? type.fields.map(cloneJson) : type.fields.map(snapshotField),
  };
}

function snapshotField(field: ParsedField): JsonObject {
  return {
    name: field.name,
    type: cloneJson(field.type),
    ...(field.amountHint === undefined ? {} : { amountHint: cloneJson(field.amountHint) }),
  };
}

function createProgramSpecV1(
  idl: ParsedIdl,
  snapshot: IdlSnapshotV1,
  hashes: {
    content: IdlContentHash;
    portable: IdlPortableHash;
    normalized: IdlNormalizedHash;
  },
): ProgramSpecV1 {
  const pdas: { [name: string]: PdaDefinitionV1 } = {};
  for (const pda of idl.pdas) {
    const name = sanitizeIdentifier(pda.name);
    pdas[name] = convertPda(name, pda);
  }
  for (const instruction of idl.instructions) {
    for (const account of flattenAccounts(instruction.accounts)) {
      if (!account.pda) continue;
      const name = sanitizeIdentifier(account.pda.name ?? account.name);
      pdas[name] ??= convertPda(name, account.pda);
    }
  }
  const usesSteel = idl.instructions.some(
    (instruction) => instruction.discriminant !== null && instruction.discriminator.length === 0,
  );
  return {
    schema: PROGRAM_SPEC_SCHEMA_V1,
    programId: idl.address,
    idlContentHash: hashes.content,
    portableIdlHash: hashes.portable,
    normalizedIdlHash: hashes.normalized,
    idlSnapshot: snapshot,
    pdas,
    instructions: idl.instructions.map((instruction) => ({
      name: instruction.name,
      discriminator: instructionDiscriminator(instruction),
      discriminator_size: usesSteel ? 1 : 8,
      accounts: flattenAccounts(instruction.accounts).map((account) => ({
        name: sanitizeIdentifier(account.name),
        is_signer: account.isSigner,
        is_writable: account.isMut,
        resolution: accountResolution(account, pdas),
        is_optional: account.optional,
        ...(account.docs.length === 0 ? {} : { docs: account.docs }),
      })),
      args: instruction.args.map((argument) => ({
        name: argument.name,
        type: idlTypeToRustString(argument.type),
        ...(argument.amountHint === undefined
          ? {}
          : { amount_hint: cloneJson(argument.amountHint) }),
      })),
      program_id: idl.address,
      ...(instruction.docs.length === 0 ? {} : { docs: instruction.docs }),
    })),
  };
}

function accountResolution(
  account: ParsedInstructionAccount,
  pdas: { [name: string]: PdaDefinitionV1 },
): AccountResolutionV1 {
  if (account.isSigner && account.address === undefined && account.pda === undefined) {
    return { category: "signer" };
  }
  if (account.address !== undefined) return { category: "known", address: account.address };
  if (account.pda !== undefined) {
    const name = sanitizeIdentifier(account.pda.name ?? account.name);
    if (Object.hasOwn(pdas, name)) return { category: "pdaRef", pda_name: name };
    const pda = convertPda(name, account.pda);
    return {
      category: "pdaInline",
      seeds: pda.seeds,
      ...(pda.program_id === undefined ? {} : { program_id: pda.program_id }),
    };
  }
  const name = sanitizeIdentifier(account.name);
  return Object.hasOwn(pdas, name)
    ? { category: "pdaRef", pda_name: name }
    : { category: "userProvided" };
}

function convertPda(name: string, pda: ParsedPda): PdaDefinitionV1 {
  const programId = pdaProgramId(pda.program);
  return {
    name,
    seeds: pda.seeds.map((seed) => {
      if (seed.kind === "account") {
        return { type: "accountRef", account_name: sanitizeSeedPath(seed.path) };
      }
      if (seed.kind === "arg") {
        return {
          type: "argRef",
          arg_name: sanitizeSeedPath(seed.path),
          ...(seed.argType === undefined ? {} : { arg_type: seed.argType }),
        };
      }
      try {
        return {
          type: "literal",
          value: new TextDecoder("utf-8", { fatal: true }).decode(Uint8Array.from(seed.value)),
        };
      } catch {
        return { type: "bytes", value: [...seed.value] };
      }
    }),
    ...(programId === undefined ? {} : { program_id: programId }),
  };
}

function pdaProgramId(program?: UnknownObject): string | undefined {
  if (!program) return undefined;
  if (typeof program.value === "string") return program.value;
  if (Array.isArray(program.value)) return encodeBase58(byteArray(program.value, "pda.program.value", true));
  return undefined;
}

function flattenAccounts(
  accounts: readonly ParsedInstructionAccount[],
  prefix?: string,
  siblingNames?: ReadonlySet<string>,
): ParsedInstructionAccount[] {
  return accounts.flatMap((source) => {
    const account = cloneParsedAccount(source);
    const flattenedName = prefix === undefined ? account.name : `${prefix}${toPascalCase(account.name)}`;
    if (account.accounts.length === 0) {
      account.name = flattenedName;
      if (account.pda && prefix !== undefined) {
        delete account.pda.name;
        for (const seed of account.pda.seeds) {
          if (seed.kind === "account" && siblingNames?.has(seed.path)) {
            seed.path = `${prefix}${toPascalCase(seed.path)}`;
          }
        }
      }
      return [account];
    }
    const children = new Set(account.accounts.map((child) => child.name));
    return flattenAccounts(account.accounts, flattenedName, children);
  });
}

function cloneParsedAccount(account: ParsedInstructionAccount): ParsedInstructionAccount {
  return {
    ...account,
    docs: [...account.docs],
    accounts: account.accounts.map(cloneParsedAccount),
    pda: account.pda
      ? {
          ...account.pda,
          seeds: account.pda.seeds.map((seed) => ({ ...seed, ...(seed.kind === "const" ? { value: [...seed.value] } : {}) })) as ParsedPdaSeed[],
          program: account.pda.program ? { ...account.pda.program } : undefined,
        }
      : undefined,
  };
}

function instructionDiscriminator(instruction: ParsedInstruction): number[] {
  if (instruction.discriminator.length > 0) return [...instruction.discriminator];
  if (instruction.discriminant !== null) return [instruction.discriminant.value & 0xff];
  return discriminator([], `global:${toSnakeCase(instruction.name)}`);
}

function discriminator(explicit: readonly number[], preimage: string): number[] {
  return explicit.length > 0
    ? [...explicit]
    : [...sha256(new TextEncoder().encode(preimage)).slice(0, 8)];
}

function idlTypeToRustString(type: JsonValue): string {
  if (typeof type === "string") {
    if (type === "string") return "String";
    if (type === "publicKey" || type === "pubkey") return "solana_pubkey::Pubkey";
    if (type === "bytes") return "Vec<u8>";
    return type;
  }
  if (!isObject(type)) return "Vec<u8>";
  if (Array.isArray(type.array) && type.array.length === 2) {
    const [element, size] = type.array;
    if (typeof size === "number") return `[${idlTypeToRustString(element ?? null)}; ${size}]`;
    return "Vec<u8>";
  }
  if (type.option !== undefined) return `Option<${idlTypeToRustString(type.option)}>`;
  if (type.vec !== undefined) return `Vec<${idlTypeToRustString(type.vec)}>`;
  if (Array.isArray(type.hashMap) && type.hashMap.length === 2) {
    return `std::collections::HashMap<${idlTypeToRustString(type.hashMap[0] ?? null)}, ${idlTypeToRustString(type.hashMap[1] ?? null)}>`;
  }
  if (typeof type.defined === "string") return type.defined;
  if (isObject(type.defined) && typeof type.defined.name === "string") return type.defined.name;
  return "Vec<u8>";
}

function sanitizeIdentifier(name: string): string {
  let output = "";
  for (const character of name) {
    if (/^[a-zA-Z0-9_]$/.test(character)) output += character;
    else if (!output.endsWith("_")) output += "_";
  }
  output = output.replace(/^_+|_+$/g, "");
  if (output.length === 0) return "value";
  return /^[0-9]/.test(output) ? `_${output}` : output;
}

function sanitizeSeedPath(path: string): string {
  return path.split(".").map(sanitizeIdentifier).join(".");
}

function toSnakeCase(value: string): string {
  let output = "";
  for (const character of value) {
    if (character.toUpperCase() === character && character.toLowerCase() !== character) {
      if (output.length > 0) output += "_";
      output += character.toLowerCase();
    } else output += character;
  }
  return output;
}

function toPascalCase(value: string): string {
  return value
    .split("_")
    .map((word) => (word.length === 0 ? "" : `${word[0]?.toUpperCase()}${word.slice(1)}`))
    .join("");
}

function encodeBase58(bytes: readonly number[]): string {
  const alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
  let value = 0n;
  for (const byte of bytes) value = value * 256n + BigInt(byte);
  let output = "";
  while (value > 0n) {
    output = `${alphabet[Number(value % 58n)]}${output}`;
    value /= 58n;
  }
  for (const byte of bytes) {
    if (byte !== 0) break;
    output = `1${output}`;
  }
  return output;
}

function cloneJson<T extends JsonValue>(value: T): T {
  if (Array.isArray(value)) return value.map((item) => cloneJson(item)) as T;
  if (isObject(value)) {
    return Object.fromEntries(Object.entries(value).map(([key, child]) => [key, cloneJson(child)])) as T;
  }
  return value;
}

function isObject(value: JsonValue | undefined): value is JsonObject {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function isObjectLike(value: unknown): value is UnknownObject {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function isParsedField(value: ParsedField | JsonValue): value is ParsedField {
  return isObjectLike(value) && typeof value.name === "string" && Object.hasOwn(value, "type");
}

function requiredObject(value: unknown, location: string): UnknownObject {
  return isObjectLike(value) ? value : invalidIdl(`${location} must be an object`);
}

function requiredString(value: unknown, location: string): string {
  return typeof value === "string" ? value : invalidIdl(`${location} must be a string`);
}

function optionalString(value: unknown, location: string): string | undefined {
  return value == null ? undefined : requiredString(value, location);
}

function optionalBoolean(value: unknown, location: string): boolean {
  return value == null
    ? false
    : typeof value === "boolean"
      ? value
      : invalidIdl(`${location} must be a boolean`);
}

function requiredArray(value: JsonValue | undefined, location: string): JsonValue[] {
  return Array.isArray(value) ? value : invalidIdl(`${location} must be an array`);
}

function optionalArray(value: JsonValue | undefined, location: string): JsonValue[] {
  return value === undefined ? [] : requiredArray(value, location);
}

function requiredUnknownArray(value: unknown, location: string): unknown[] {
  return Array.isArray(value) ? value : invalidIdl(`${location} must be an array`);
}

function optionalUnknownArray(value: unknown, location: string): unknown[] {
  return value == null ? [] : requiredUnknownArray(value, location);
}

function stringArray(value: unknown, location: string): string[] {
  return optionalUnknownArray(value, location).map((item, index) =>
    requiredString(item, `${location}[${index}]`),
  );
}

function byteArray(value: unknown, location: string, required = false): number[] {
  if (value == null && !required) return [];
  return requiredUnknownArray(value, location).map((item, index) => {
    if (typeof item !== "number" || !Number.isInteger(item) || item < 0 || item > 255) {
      return invalidIdl(`${location}[${index}] must be a byte`);
    }
    return item;
  });
}

function invalidIdl(message: string): never {
  return hashError("invalid-idl", `failed to parse IDL: ${message}`);
}
