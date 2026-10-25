/**
 * Validation for `arete.protocol-spec/v1`, mirroring
 * `arete-hash/src/protocol/spec.rs` check for check.
 *
 * Validation runs in two passes:
 *
 * 1. **Shape**: every field, type, bound and identifier, in document order.
 * 2. **Semantics**: references between sections and the static checks on
 *    machines, steps, roles, reactions and events.
 *
 * Expression sources are only bounded here; `check/expressions.ts` gives
 * authors early reference feedback, and the platform type-checks them.
 */

import { isHashId } from "@usearete/hash";

import {
  BUILTIN_PROGRAMS,
  BUILTIN_PROGRAM_IDS,
  PROTOCOL_CTX_V1,
  PROTOCOL_HANDLER_ABI_V1,
  PROTOCOL_SPEC_SCHEMA_V1,
  STRING_FORMATS,
  isClientCallable,
  type BuiltinProgram,
  type FieldSchemaV1,
  type MachineKind,
  type MachineV1,
  type ProtocolSpecV1,
  type StepV1,
  type TriggerV1,
} from "./types.js";
import {
  MAX_IDENTIFIER_BYTES,
  MAX_SAFE,
  Obj,
  asArray,
  asBool,
  asInteger,
  asMap,
  asStr,
  boundedStr,
  description,
  err,
  eventName,
  expression,
  identifierValue,
  index,
  isRustWhitespace,
  keyEvent,
  keyIdentifier,
  keySnake,
  oneOf,
  protocolName,
  pubkey,
  rejectFloats,
  schemaTag,
  semver,
  snakeValue,
  sortedKeys,
  tagged,
  uniqueStrings,
} from "./shape.js";

const MAX_PROGRAMS = 32;
const MAX_STACKS = 16;
const MAX_ROLES = 16;
const MAX_RESOURCES = 32;
const MAX_READS = 64;
const MAX_STATES = 64;
const MAX_DERIVE_RULES = 64;
const MAX_STEPS = 64;
const MAX_GUARDS = 32;
const MAX_EPHEMERAL = 8;
const MAX_TRANSITIONS = 16;
const MAX_BIND_ACCOUNTS = 32;
const MAX_ATAS = 8;
const MAX_TEMPLATE_INSTRUCTIONS = 64;
const MAX_TEMPLATE_FIELDS = 64;
const MAX_REMAINING_ACCOUNTS = 32;
const MAX_REACTIONS = 128;
/**
 * Rejection codes a prepare returns for access refusals and terminal
 * lifecycle states. A guard may not use them, so a client can always tell a
 * guard failure from these.
 */
export const RESERVED_GUARD_CODES: readonly string[] = [
  "step_requires_secret_key",
  "step_not_in_session_scope",
  "step_not_client_callable",
  "instance_terminal",
  "participant_terminal",
];
const MAX_EVENTS = 64;
const MAX_SCHEMA_PROPERTIES = 64;
const MAX_SCHEMA_DEPTH = 4;
const MAX_ENUM_VALUES = 64;
const MAX_METADATA_FIELDS = 64;
const MAX_STACK_FIELDS = 64;
const MAX_ACTIVITY = 64;
const MAX_TITLE_BYTES = 128;

export function hashIdValue(value: unknown, path: string, kind: Parameters<typeof isHashId>[1]): string {
  const text = asStr(value, path);
  if (!isHashId(text, kind)) {
    err("invalid-hash-id", path, `expected an arete:h1:${kind}:sha256 identifier`);
  }
  return text;
}

/** Validate a ProtocolSpec JSON value and return it as the typed model. */
export function validateProtocolSpec(value: unknown): ProtocolSpecV1 {
  rejectFloats(value, "");
  shapeSpec(value);
  const spec = value as ProtocolSpecV1;
  checkSemantics(spec);
  return spec;
}

// ---------------------------------------------------------------------------
// Shape
// ---------------------------------------------------------------------------

const SPEC_KEYS = [
  "schema",
  "name",
  "version",
  "description",
  "handlerAbi",
  "ctxVersion",
  "programs",
  "stacks",
  "params",
  "roles",
  "resources",
  "instanceVisibility",
  "reads",
  "machines",
  "steps",
  "reactions",
  "events",
  "metadata",
  "stack",
];

function shapeSpec(value: unknown): void {
  const root = new Obj(value, "", SPEC_KEYS);
  schemaTag(root.req("schema"), root.pathOf("schema"), PROTOCOL_SPEC_SCHEMA_V1);
  protocolName(asStr(root.req("name"), root.pathOf("name")), root.pathOf("name"));
  semver(root.req("version"), root.pathOf("version"));
  const descriptionValue = root.opt("description");
  if (descriptionValue !== undefined) description(descriptionValue, root.pathOf("description"));
  const abi = root.opt("handlerAbi");
  if (abi !== undefined) oneOf(abi, root.pathOf("handlerAbi"), [PROTOCOL_HANDLER_ABI_V1]);
  const ctx = root.opt("ctxVersion");
  if (ctx !== undefined) oneOf(ctx, root.pathOf("ctxVersion"), [PROTOCOL_CTX_V1]);
  shapePrograms(root.req("programs"), root.pathOf("programs"));
  shapeStacks(root.req("stacks"), root.pathOf("stacks"));
  shapeObjectSchema(root.req("params"), root.pathOf("params"));
  for (const [, role, path] of asMap(root.req("roles"), root.pathOf("roles"), MAX_ROLES, keyIdentifier)) {
    const obj = new Obj(role, path, ["description"]);
    const text = obj.opt("description");
    if (text !== undefined) description(text, obj.pathOf("description"));
  }
  for (const [, resource, path] of asMap(
    root.req("resources"),
    root.pathOf("resources"),
    MAX_RESOURCES,
    keyIdentifier,
  )) {
    const obj = new Obj(resource, path, ["schema", "description"]);
    shapeFieldSchema(obj.req("schema"), obj.pathOf("schema"), 1);
    const text = obj.opt("description");
    if (text !== undefined) description(text, obj.pathOf("description"));
  }
  oneOf(root.req("instanceVisibility"), root.pathOf("instanceVisibility"), ["shared", "account"]);
  for (const [, read, path] of asMap(root.req("reads"), root.pathOf("reads"), MAX_READS, keyIdentifier)) {
    shapeRead(read, path);
  }
  const machines = new Obj(root.req("machines"), root.pathOf("machines"), ["instance", "participant"]);
  shapeMachine(machines.req("instance"), machines.pathOf("instance"));
  const participant = machines.opt("participant");
  if (participant !== undefined) shapeMachine(participant, machines.pathOf("participant"));
  for (const [, step, path] of asMap(root.req("steps"), root.pathOf("steps"), MAX_STEPS, keyIdentifier)) {
    shapeStep(step, path);
  }
  const reactionsPath = root.pathOf("reactions");
  asArray(root.req("reactions"), reactionsPath, MAX_REACTIONS).forEach((reaction, i) =>
    shapeReaction(reaction, index(reactionsPath, i)),
  );
  for (const [, event, path] of asMap(root.req("events"), root.pathOf("events"), MAX_EVENTS, keyEvent)) {
    const obj = new Obj(event, path, ["machine", "description"]);
    oneOf(obj.req("machine"), obj.pathOf("machine"), ["instance", "participant"]);
    const text = obj.opt("description");
    if (text !== undefined) description(text, obj.pathOf("description"));
  }
  shapeMetadata(root.req("metadata"), root.pathOf("metadata"));
  shapeStackSurface(root.req("stack"), root.pathOf("stack"));
}

function shapePrograms(value: unknown, path: string): void {
  const programIds = new Map<string, string>();
  for (const [alias, program, programPath] of asMap(value, path, MAX_PROGRAMS, keyIdentifier)) {
    const [kind, obj] = tagged(program, programPath, [
      ["programSpec", ["programId", "programSpecHash", "programReleaseHash"]],
      ["builtin", ["builtin"]],
    ]);
    let id: string;
    let idPath: string;
    if (kind === "programSpec") {
      id = pubkey(obj.req("programId"), obj.pathOf("programId"));
      hashIdValue(obj.req("programSpecHash"), obj.pathOf("programSpecHash"), "program-spec");
      const release = obj.opt("programReleaseHash");
      if (release !== undefined) {
        hashIdValue(release, obj.pathOf("programReleaseHash"), "program-release");
      }
      idPath = obj.pathOf("programId");
    } else {
      const builtin = oneOf(obj.req("builtin"), obj.pathOf("builtin"), BUILTIN_PROGRAMS);
      id = BUILTIN_PROGRAM_IDS[builtin as BuiltinProgram];
      idPath = obj.pathOf("builtin");
    }
    const previous = programIds.get(id);
    if (previous !== undefined) {
      err("duplicate-entry", idPath, `program ${id} is already declared as '${previous}'`);
    }
    programIds.set(id, alias);
  }
}

function shapeStacks(value: unknown, path: string): void {
  for (const [, stack, stackPath] of asMap(value, path, MAX_STACKS, keyIdentifier)) {
    const obj = new Obj(stack, stackPath, ["stackManifestHash", "package"]);
    hashIdValue(obj.req("stackManifestHash"), obj.pathOf("stackManifestHash"), "stack-manifest");
    const pkg = obj.opt("package");
    if (pkg !== undefined) boundedStr(pkg, obj.pathOf("package"), MAX_TITLE_BYTES);
  }
}

const FIELD_SCHEMA_KEYS = [
  "type",
  "format",
  "enum",
  "maxLength",
  "minimum",
  "maximum",
  "items",
  "maxItems",
  "properties",
  "required",
  "additionalProperties",
  "description",
] as const;

function shapeObjectSchema(value: unknown, path: string): void {
  const schema = new Obj(value, path, ["type", "properties", "required", "additionalProperties"]);
  shapeObjectBody(schema, 1);
}

function shapeObjectBody(schema: Obj, depth: number): void {
  const typePath = schema.pathOf("type");
  if (asStr(schema.req("type"), typePath) !== "object") {
    err("invalid-schema", typePath, "expected type 'object'");
  }
  const properties = asMap(
    schema.req("properties"),
    schema.pathOf("properties"),
    MAX_SCHEMA_PROPERTIES,
    keyIdentifier,
  );
  for (const [, property, propertyPath] of properties) {
    shapeFieldSchema(property, propertyPath, depth + 1);
  }
  const names = new Set(properties.map(([name]) => name));
  const requiredPath = schema.pathOf("required");
  const required = uniqueStrings(schema.req("required"), requiredPath, MAX_SCHEMA_PROPERTIES, identifierValue);
  required.forEach((name, i) => {
    if (!names.has(name)) {
      err("invalid-schema", index(requiredPath, i), `required property '${name}' is not declared`);
    }
  });
  const additionalPath = schema.pathOf("additionalProperties");
  if (asBool(schema.req("additionalProperties"), additionalPath)) {
    err("invalid-schema", additionalPath, "additionalProperties must be false");
  }
}

function shapeFieldSchema(value: unknown, path: string, depth: number): void {
  if (depth > MAX_SCHEMA_DEPTH) {
    err("invalid-schema", path, `schemas nest at most ${MAX_SCHEMA_DEPTH} levels`);
  }
  const schema = new Obj(value, path, FIELD_SCHEMA_KEYS);
  const fieldType = oneOf(schema.req("type"), schema.pathOf("type"), [
    "string",
    "boolean",
    "integer",
    "array",
    "object",
  ]);
  const allowed: readonly string[] =
    fieldType === "string"
      ? ["type", "format", "enum", "maxLength", "description"]
      : fieldType === "boolean"
        ? ["type", "description"]
        : fieldType === "integer"
          ? ["type", "minimum", "maximum", "description"]
          : fieldType === "array"
            ? ["type", "items", "maxItems", "description"]
            : ["type", "properties", "required", "additionalProperties", "description"];
  for (const key of FIELD_SCHEMA_KEYS) {
    if (schema.has(key) && !allowed.includes(key)) {
      err("invalid-schema", schema.pathOf(key), `'${key}' is not allowed on type '${fieldType}'`);
    }
  }
  switch (fieldType) {
    case "string": {
      const format = schema.opt("format");
      if (format !== undefined) oneOf(format, schema.pathOf("format"), STRING_FORMATS);
      const enumValue = schema.opt("enum");
      if (enumValue !== undefined) {
        const enumPath = schema.pathOf("enum");
        if (schema.has("format")) {
          err("invalid-schema", enumPath, "'enum' cannot be combined with 'format'");
        }
        const values = uniqueStrings(enumValue, enumPath, MAX_ENUM_VALUES, (item, itemPath) => {
          const text = boundedStr(item, itemPath, MAX_IDENTIFIER_BYTES);
          if (text.length === 0) err("invalid-value", itemPath, "enum values must not be empty");
          return text;
        });
        if (values.length === 0) err("invalid-schema", enumPath, "'enum' must list at least one value");
      }
      const maxLength = schema.opt("maxLength");
      if (maxLength !== undefined) asInteger(maxLength, schema.pathOf("maxLength"), 1, 4096);
      break;
    }
    case "integer": {
      const minimumValue = schema.opt("minimum");
      const minimum =
        minimumValue === undefined
          ? undefined
          : asInteger(minimumValue, schema.pathOf("minimum"), -MAX_SAFE, MAX_SAFE);
      const maximumValue = schema.opt("maximum");
      const maximum =
        maximumValue === undefined
          ? undefined
          : asInteger(maximumValue, schema.pathOf("maximum"), -MAX_SAFE, MAX_SAFE);
      if (minimum !== undefined && maximum !== undefined && minimum > maximum) {
        err("invalid-schema", schema.pathOf("maximum"), "maximum is below minimum");
      }
      break;
    }
    case "array":
      shapeFieldSchema(schema.req("items"), schema.pathOf("items"), depth + 1);
      asInteger(schema.req("maxItems"), schema.pathOf("maxItems"), 1, 256);
      break;
    case "object":
      shapeObjectBody(schema, depth);
      break;
    default:
      break;
  }
  const text = schema.opt("description");
  if (text !== undefined) description(text, schema.pathOf("description"));
}

function shapeRead(value: unknown, path: string): void {
  const [kind, read] = tagged(value, path, [
    ["account", ["program", "account", "address", "optional"]],
    ["tokenAccount", ["address", "optional"]],
    ["lamports", ["address"]],
  ]);
  if (kind === "account") {
    identifierValue(read.req("program"), read.pathOf("program"));
    identifierValue(read.req("account"), read.pathOf("account"));
  }
  expression(read.req("address"), read.pathOf("address"));
  if (kind !== "lamports") asBool(read.req("optional"), read.pathOf("optional"));
}

function shapeMachine(value: unknown, path: string): void {
  const machine = new Obj(value, path, ["initial", "states", "derive", "reads"]);
  snakeValue(machine.req("initial"), machine.pathOf("initial"));
  const states = asMap(machine.req("states"), machine.pathOf("states"), MAX_STATES, keySnake);
  if (states.length === 0) {
    err("invalid-value", machine.pathOf("states"), "a machine needs at least one state");
  }
  for (const [, state, statePath] of states) {
    const obj = new Obj(state, statePath, ["kind", "terminal", "description"]);
    oneOf(obj.req("kind"), obj.pathOf("kind"), ["recorded", "derived"]);
    asBool(obj.req("terminal"), obj.pathOf("terminal"));
    const text = obj.opt("description");
    if (text !== undefined) description(text, obj.pathOf("description"));
  }
  const derivePath = machine.pathOf("derive");
  asArray(machine.req("derive"), derivePath, MAX_DERIVE_RULES).forEach((item, i) => {
    const rule = new Obj(item, index(derivePath, i), ["from", "to", "when", "at"]);
    const from = uniqueStrings(rule.req("from"), rule.pathOf("from"), MAX_STATES, snakeValue);
    if (from.length === 0) {
      err("invalid-value", rule.pathOf("from"), "a derivation needs at least one source state");
    }
    snakeValue(rule.req("to"), rule.pathOf("to"));
    expression(rule.req("when"), rule.pathOf("when"));
    const at = rule.opt("at");
    if (at !== undefined) expression(at, rule.pathOf("at"));
  });
  uniqueStrings(machine.req("reads"), machine.pathOf("reads"), MAX_READS, identifierValue);
}

const STEP_KEYS = [
  "scope",
  "title",
  "description",
  "authority",
  "access",
  "args",
  "guards",
  "reads",
  "ephemeral",
  "fee",
  "transitions",
  "bind",
  "glue",
  "build",
];

function shapeStep(value: unknown, path: string): void {
  const step = new Obj(value, path, STEP_KEYS);
  oneOf(step.req("scope"), step.pathOf("scope"), ["create", "instance", "participant"]);
  const title = step.opt("title");
  if (title !== undefined) boundedStr(title, step.pathOf("title"), MAX_TITLE_BYTES);
  const text = step.opt("description");
  if (text !== undefined) description(text, step.pathOf("description"));
  const [kind, authority] = tagged(step.req("authority"), step.pathOf("authority"), [
    ["creator", []],
    ["participant", []],
    ["role", ["role"]],
    ["keeper", []],
    ["squadsProposer", ["multisig", "vaultIndex"]],
  ]);
  if (kind === "role") identifierValue(authority.req("role"), authority.pathOf("role"));
  if (kind === "squadsProposer") {
    expression(authority.req("multisig"), authority.pathOf("multisig"));
    const vaultIndex = authority.opt("vaultIndex");
    if (vaultIndex !== undefined && asInteger(vaultIndex, authority.pathOf("vaultIndex"), 0, 255) === 0) {
      err("invalid-value", authority.pathOf("vaultIndex"), "omit vaultIndex for vault 0 instead of giving 0");
    }
  }
  const access = step.opt("access");
  if (access !== undefined) oneOf(access, step.pathOf("access"), ["public", "secret"]);
  shapeObjectSchema(step.req("args"), step.pathOf("args"));
  const guardsPath = step.pathOf("guards");
  asArray(step.req("guards"), guardsPath, MAX_GUARDS).forEach((item, i) => {
    const guard = new Obj(item, index(guardsPath, i), ["code", "expr", "message"]);
    snakeValue(guard.req("code"), guard.pathOf("code"));
    expression(guard.req("expr"), guard.pathOf("expr"));
    const message = guard.opt("message");
    if (message !== undefined) description(message, guard.pathOf("message"));
  });
  uniqueStrings(step.req("reads"), step.pathOf("reads"), MAX_READS, identifierValue);
  uniqueStrings(step.req("ephemeral"), step.pathOf("ephemeral"), MAX_EPHEMERAL, identifierValue);
  shapeFee(step.req("fee"), step.pathOf("fee"));
  const transitionsPath = step.pathOf("transitions");
  asArray(step.req("transitions"), transitionsPath, MAX_TRANSITIONS).forEach((item, i) => {
    const transition = new Obj(item, index(transitionsPath, i), ["machine", "from", "to"]);
    oneOf(transition.req("machine"), transition.pathOf("machine"), ["instance", "participant"]);
    uniqueStrings(transition.req("from"), transition.pathOf("from"), MAX_STATES, snakeValue);
    snakeValue(transition.req("to"), transition.pathOf("to"));
  });
  const bindValue = step.opt("bind");
  if (bindValue !== undefined) {
    const bind = new Obj(bindValue, step.pathOf("bind"), ["accounts", "roles"]);
    for (const [, expr, exprPath] of asMap(
      bind.req("accounts"),
      bind.pathOf("accounts"),
      MAX_BIND_ACCOUNTS,
      keyIdentifier,
    )) {
      expression(expr, exprPath);
    }
    for (const [, expr, exprPath] of asMap(bind.req("roles"), bind.pathOf("roles"), MAX_ROLES, keyIdentifier)) {
      expression(expr, exprPath);
    }
  }
  const glue = new Obj(step.req("glue"), step.pathOf("glue"), ["atas", "wrapSol", "unwrapSol"]);
  const atasPath = glue.pathOf("atas");
  asArray(glue.req("atas"), atasPath, MAX_ATAS).forEach((item, i) => {
    const ata = new Obj(item, index(atasPath, i), ["owner", "mint"]);
    expression(ata.req("owner"), ata.pathOf("owner"));
    expression(ata.req("mint"), ata.pathOf("mint"));
  });
  const wrapValue = glue.opt("wrapSol");
  if (wrapValue !== undefined) {
    const wrap = new Obj(wrapValue, glue.pathOf("wrapSol"), ["amount"]);
    expression(wrap.req("amount"), wrap.pathOf("amount"));
  }
  asBool(glue.req("unwrapSol"), glue.pathOf("unwrapSol"));
  const [buildKind, build] = tagged(step.req("build"), step.pathOf("build"), [
    ["template", ["instructions"]],
    ["handler", ["export", "programs"]],
  ]);
  if (buildKind === "template") {
    const instructionsPath = build.pathOf("instructions");
    // An empty template is a state-only step: its transaction carries only
    // the attested memo (and any glue), and its transitions apply when that
    // transaction confirms.
    const instructions = asArray(build.req("instructions"), instructionsPath, MAX_TEMPLATE_INSTRUCTIONS);
    if (instructions.length === 0 && kind === "squadsProposer") {
      err("invalid-value", instructionsPath, "a squadsProposer step proposes its instructions, so it needs at least one");
    }
    instructions.forEach((item, i) => {
      const instruction = new Obj(item, index(instructionsPath, i), [
        "program",
        "instruction",
        "args",
        "accounts",
        "remainingAccounts",
      ]);
      identifierValue(instruction.req("program"), instruction.pathOf("program"));
      identifierValue(instruction.req("instruction"), instruction.pathOf("instruction"));
      for (const key of ["args", "accounts"]) {
        for (const [, expr, exprPath] of asMap(
          instruction.req(key),
          instruction.pathOf(key),
          MAX_TEMPLATE_FIELDS,
          keyIdentifier,
        )) {
          expression(expr, exprPath);
        }
      }
      const remaining = instruction.opt("remainingAccounts");
      if (remaining !== undefined) shapeRemainingAccounts(remaining, instruction.pathOf("remainingAccounts"));
    });
  } else {
    identifierValue(build.req("export"), build.pathOf("export"));
    uniqueStrings(build.req("programs"), build.pathOf("programs"), MAX_PROGRAMS, identifierValue);
  }
}

/** `remainingAccounts`: a non-empty list (omitted when there are none). Remaining accounts never sign. */
function shapeRemainingAccounts(value: unknown, path: string): void {
  const accounts = asArray(value, path, MAX_REMAINING_ACCOUNTS);
  if (accounts.length === 0) err("invalid-value", path, "omit remainingAccounts instead of giving an empty list");
  accounts.forEach((item, i) => {
    const account = new Obj(item, index(path, i), ["pubkey", "isWritable", "isSigner"]);
    expression(account.req("pubkey"), account.pathOf("pubkey"));
    asBool(account.req("isWritable"), account.pathOf("isWritable"));
    if (asBool(account.req("isSigner"), account.pathOf("isSigner"))) {
      err(
        "remaining-account-signer",
        account.pathOf("isSigner"),
        "remaining accounts cannot sign; name signers as IDL accounts",
      );
    }
  });
}

function shapeFee(value: unknown, path: string): void {
  const [kind, fee] = tagged(value, path, [
    ["none", []],
    ["input", ["amountArg", "asset"]],
    ["fixed", []],
  ]);
  if (kind === "input") {
    identifierValue(fee.req("amountArg"), fee.pathOf("amountArg"));
    const [assetKind, asset] = tagged(fee.req("asset"), fee.pathOf("asset"), [
      ["native", []],
      ["spl", ["mint", "sourceArg"]],
    ]);
    if (assetKind === "spl") {
      expression(asset.req("mint"), asset.pathOf("mint"));
      identifierValue(asset.req("sourceArg"), asset.pathOf("sourceArg"));
    }
  }
}

function shapeReaction(value: unknown, path: string): void {
  const reaction = new Obj(value, path, ["id", "trigger", "action"]);
  identifierValue(reaction.req("id"), reaction.pathOf("id"));
  const [kind, trigger] = tagged(reaction.req("trigger"), reaction.pathOf("trigger"), [
    ["at", ["machine", "in", "time"]],
    ["when", ["machine", "in", "condition"]],
    ["onTransition", ["machine", "from", "to"]],
    ["onConfirmed", ["step"]],
  ]);
  if (kind === "at" || kind === "when") {
    oneOf(trigger.req("machine"), trigger.pathOf("machine"), ["instance", "participant"]);
    const states = uniqueStrings(trigger.req("in"), trigger.pathOf("in"), MAX_STATES, snakeValue);
    if (states.length === 0) err("invalid-value", trigger.pathOf("in"), "list at least one state");
    const key = kind === "at" ? "time" : "condition";
    expression(trigger.req(key), trigger.pathOf(key));
  } else if (kind === "onTransition") {
    oneOf(trigger.req("machine"), trigger.pathOf("machine"), ["instance", "participant"]);
    uniqueStrings(trigger.req("from"), trigger.pathOf("from"), MAX_STATES, snakeValue);
    const to = uniqueStrings(trigger.req("to"), trigger.pathOf("to"), MAX_STATES, snakeValue);
    if (to.length === 0) err("invalid-value", trigger.pathOf("to"), "list at least one state");
  } else {
    identifierValue(trigger.req("step"), trigger.pathOf("step"));
  }
  const [actionKind, action] = tagged(reaction.req("action"), reaction.pathOf("action"), [
    ["runKeeperStep", ["step"]],
    ["proposeSquads", ["step"]],
    ["emit", ["event"]],
  ]);
  if (actionKind === "emit") {
    const eventPath = action.pathOf("event");
    eventName(asStr(action.req("event"), eventPath), eventPath);
  } else {
    identifierValue(action.req("step"), action.pathOf("step"));
  }
}

function shapeMetadata(value: unknown, path: string): void {
  const metadata = new Obj(value, path, ["instance", "participant"]);
  for (const machine of ["instance", "participant"] as const) {
    const schemaValue = metadata.opt(machine);
    if (schemaValue === undefined) continue;
    const schema = new Obj(schemaValue, metadata.pathOf(machine), ["fields", "maxBytes"]);
    for (const [, fieldValue, fieldPath] of asMap(
      schema.req("fields"),
      schema.pathOf("fields"),
      MAX_METADATA_FIELDS,
      keyIdentifier,
    )) {
      const field = new Obj(fieldValue, fieldPath, ["schema", "writer", "filterable"]);
      shapeFieldSchema(field.req("schema"), field.pathOf("schema"), 1);
      const writers = machine === "instance" ? ["creator"] : ["creator", "participant"];
      oneOf(field.req("writer"), field.pathOf("writer"), writers);
      asBool(field.req("filterable"), field.pathOf("filterable"));
    }
    asInteger(schema.req("maxBytes"), schema.pathOf("maxBytes"), 1, 65_536);
  }
}

function fieldPathValue(value: unknown, path: string): string {
  const text = boundedStr(value, path, 128);
  const valid =
    text.length > 0 &&
    text.split(".").every((segment) => /^[A-Za-z_][A-Za-z0-9_]*$/.test(segment));
  if (!valid) err("invalid-identifier", path, `'${text}' must be a dotted field path`);
  return text;
}

function shapeStackSurface(value: unknown, path: string): void {
  const stack = new Obj(value, path, ["instanceAccounts", "instanceFields", "activity"]);
  for (const [, account, accountPath] of asMap(
    stack.req("instanceAccounts"),
    stack.pathOf("instanceAccounts"),
    MAX_BIND_ACCOUNTS,
    keyIdentifier,
  )) {
    const obj = new Obj(account, accountPath, ["program", "account"]);
    identifierValue(obj.req("program"), obj.pathOf("program"));
    identifierValue(obj.req("account"), obj.pathOf("account"));
  }
  for (const [, field, fieldPath] of asMap(
    stack.req("instanceFields"),
    stack.pathOf("instanceFields"),
    MAX_STACK_FIELDS,
    keyIdentifier,
  )) {
    const obj = new Obj(field, fieldPath, ["account", "field"]);
    identifierValue(obj.req("account"), obj.pathOf("account"));
    fieldPathValue(obj.req("field"), obj.pathOf("field"));
  }
  const activityPath = stack.pathOf("activity");
  asArray(stack.req("activity"), activityPath, MAX_ACTIVITY).forEach((item, i) => {
    const activity = new Obj(item, index(activityPath, i), [
      "step",
      "program",
      "instruction",
      "instanceAccount",
      "walletAccount",
      "amountArg",
    ]);
    for (const key of ["step", "program", "instruction", "instanceAccount", "walletAccount"]) {
      identifierValue(activity.req(key), activity.pathOf(key));
    }
    const amount = activity.opt("amountArg");
    if (amount !== undefined) identifierValue(amount, activity.pathOf("amountArg"));
  });
}

// ---------------------------------------------------------------------------
// Semantics
// ---------------------------------------------------------------------------

/** Entries of a record in Rust `BTreeMap` order. */
function entries<T>(record: Record<string, T>): Array<[string, T]> {
  return sortedKeys(record).map((key) => [key, record[key]!]);
}

function has(record: Record<string, unknown>, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(record, key);
}

export function hasHandlers(spec: ProtocolSpecV1): boolean {
  return Object.values(spec.steps).some((step) => step.build.kind === "handler");
}

export function machineOf(spec: ProtocolSpecV1, kind: MachineKind): MachineV1 | undefined {
  return kind === "instance" ? spec.machines.instance : spec.machines.participant;
}

export function createStep(spec: ProtocolSpecV1): [string, StepV1] | undefined {
  return entries(spec.steps).find(([, step]) => step.scope === "create");
}

function checkSemantics(spec: ProtocolSpecV1): void {
  checkHandlerAbi(spec);
  checkReads(spec);
  checkMachine(spec, "instance");
  checkParticipantMachinePresent(spec);
  if (spec.machines.participant !== undefined) checkMachine(spec, "participant");
  checkSteps(spec);
  checkStackSurface(spec);
  checkReachability(spec, "instance");
  if (spec.machines.participant !== undefined) checkReachability(spec, "participant");
  checkReactions(spec);
  checkEvents(spec);
}

function checkHandlerAbi(spec: ProtocolSpecV1): void {
  const handlers = hasHandlers(spec);
  if (handlers && spec.handlerAbi === undefined) {
    err("handler-abi-required", "/handlerAbi", "handler steps need handlerAbi");
  }
  if (!handlers && spec.handlerAbi !== undefined) {
    err("handler-abi-unused", "/handlerAbi", "no step has a handler");
  }
  if (handlers && spec.ctxVersion === undefined) {
    err("handler-abi-required", "/ctxVersion", "handler steps need ctxVersion");
  }
  if (!handlers && spec.ctxVersion !== undefined) {
    err("handler-abi-unused", "/ctxVersion", "no step has a handler");
  }
}

function specProgram(spec: ProtocolSpecV1, alias: string, path: string) {
  if (!has(spec.programs, alias)) err("unknown-program", path, `program '${alias}' is not declared`);
  return spec.programs[alias]!;
}

function requireProgramSpec(spec: ProtocolSpecV1, alias: string, path: string): void {
  if (specProgram(spec, alias, path).kind === "builtin") {
    err("unknown-program", path, `program '${alias}' is a builtin and has no ProgramSpec`);
  }
}

function checkReads(spec: ProtocolSpecV1): void {
  for (const [name, read] of entries(spec.reads)) {
    if (read.kind === "account") requireProgramSpec(spec, read.program, `/reads/${name}/program`);
  }
}

function requireRead(spec: ProtocolSpecV1, name: string, path: string): void {
  if (!has(spec.reads, name)) err("unknown-read", path, `read '${name}' is not declared`);
}

function requireState(machine: MachineV1, state: string, path: string): void {
  if (!has(machine.states, state)) err("unknown-state", path, `state '${state}' is not declared`);
}

function checkMachine(spec: ProtocolSpecV1, kind: MachineKind): void {
  const machine = machineOf(spec, kind)!;
  const base = `/machines/${kind}`;
  requireState(machine, machine.initial, `${base}/initial`);
  machine.derive.forEach((rule, i) => {
    const rulePath = `${base}/derive/${i}`;
    rule.from.forEach((from, j) => requireState(machine, from, `${rulePath}/from/${j}`));
    requireState(machine, rule.to, `${rulePath}/to`);
    if (machine.states[rule.to]!.kind !== "derived") {
      err(
        "derive-targets-recorded-state",
        `${rulePath}/to`,
        `'${rule.to}' is a recorded state; derivations may only target derived states`,
      );
    }
  });
  machine.reads.forEach((read, i) => requireRead(spec, read, `${base}/reads/${i}`));
  for (const [name, state] of entries(machine.states)) {
    if (
      state.kind === "derived" &&
      name !== machine.initial &&
      !machine.derive.some((rule) => rule.to === name)
    ) {
      err("derived-state-without-rule", `${base}/states/${name}`, `no derivation targets derived state '${name}'`);
    }
  }
}

function usesParticipantMachine(spec: ProtocolSpecV1): string | undefined {
  for (const [name, step] of entries(spec.steps)) {
    if (step.scope === "participant") return `/steps/${name}/scope`;
    const i = step.transitions.findIndex((transition) => transition.machine === "participant");
    if (i >= 0) return `/steps/${name}/transitions/${i}/machine`;
  }
  for (let i = 0; i < spec.reactions.length; i += 1) {
    const trigger = spec.reactions[i]!.trigger;
    if (trigger.kind !== "onConfirmed" && trigger.machine === "participant") {
      return `/reactions/${i}/trigger/machine`;
    }
  }
  for (const [name, event] of entries(spec.events)) {
    if (event.machine === "participant") return `/events/${name}/machine`;
  }
  if (spec.metadata.participant !== undefined) return "/metadata/participant";
  return undefined;
}

function checkParticipantMachinePresent(spec: ProtocolSpecV1): void {
  if (spec.machines.participant === undefined) {
    const path = usesParticipantMachine(spec);
    if (path !== undefined) {
      err("missing-participant-machine", path, "participant scope needs machines.participant");
    }
  }
}

function compact(text: string): string {
  return Array.from(text)
    .filter((c) => !isRustWhitespace(c.codePointAt(0)!))
    .join("");
}

function roleGuardPresent(step: StepV1, role: string): boolean {
  const forward = `wallet==instance.roles.${role}`;
  const reverse = `instance.roles.${role}==wallet`;
  return step.guards.some((guard) => {
    const expr = compact(guard.expr);
    return expr === forward || expr === reverse;
  });
}

function argSchema(step: StepV1, name: string): FieldSchemaV1 | undefined {
  return has(step.args.properties, name) ? step.args.properties[name] : undefined;
}

function checkSteps(spec: ProtocolSpecV1): void {
  const creates = entries(spec.steps)
    .filter(([, step]) => step.scope === "create")
    .map(([name]) => name);
  if (creates.length === 0) err("create-step-count", "/steps", "exactly one create step is required");
  if (creates.length > 1) {
    err("create-step-count", `/steps/${creates[1]}/scope`, "exactly one create step is allowed");
  }
  for (const [name, step] of entries(spec.steps)) {
    const base = `/steps/${name}`;
    const authorityPath = `${base}/authority`;
    const authority = step.authority;
    let authorityOk = false;
    if (authority.kind === "creator") authorityOk = step.scope === "create";
    else if (authority.kind === "participant") authorityOk = step.scope === "participant";
    else if (authority.kind === "role") {
      if (step.scope === "instance" || step.scope === "participant") {
        if (!has(spec.roles, authority.role)) {
          err("unknown-role", `${authorityPath}/role`, `role '${authority.role}' is not declared`);
        }
        authorityOk = true;
      }
    } else if (authority.kind === "keeper") {
      authorityOk = step.scope === "instance" || step.scope === "participant";
    } else {
      authorityOk = step.scope === "instance";
    }
    if (!authorityOk) {
      err("invalid-authority", authorityPath, "the authority is not allowed for this step scope");
    }
    const callable = isClientCallable(authority);
    if (callable && step.access === undefined) {
      err("missing-access", `${base}/access`, "client-callable steps declare access 'public' or 'secret'");
    }
    if (!callable && step.access !== undefined) {
      err("forbidden-access", `${base}/access`, "keeper and admin steps are never client-callable");
    }
    const codes = new Set<string>();
    step.guards.forEach((guard, i) => {
      if (RESERVED_GUARD_CODES.includes(guard.code)) {
        err(
          "reserved-guard-code",
          `${base}/guards/${i}/code`,
          `guard code '${guard.code}' is reserved: prepares return it for access or lifecycle rejections`,
        );
      }
      if (codes.has(guard.code)) {
        err("duplicate-guard-code", `${base}/guards/${i}/code`, `guard code '${guard.code}' is used twice`);
      }
      codes.add(guard.code);
    });
    if (authority.kind === "role" && !roleGuardPresent(step, authority.role)) {
      err(
        "missing-role-guard",
        `${base}/guards`,
        `role steps need the guard 'wallet == instance.roles.${authority.role}'`,
      );
    }
    step.reads.forEach((read, i) => requireRead(spec, read, `${base}/reads/${i}`));
    checkFee(step, base);
    checkTransitions(spec, step, base);
    checkBind(spec, step, base);
    if (step.build.kind === "template") {
      step.build.instructions.forEach((instruction, i) =>
        specProgram(spec, instruction.program, `${base}/build/instructions/${i}/program`),
      );
    } else {
      step.build.programs.forEach((program, i) =>
        specProgram(spec, program, `${base}/build/programs/${i}`),
      );
    }
  }
}

function checkFee(step: StepV1, base: string): void {
  const feePath = `${base}/fee`;
  const fee = step.fee;
  if (fee.kind === "none") return;
  if (!isClientCallable(step.authority)) {
    err("invalid-fee-basis", feePath, "only wallet steps carry fees");
  }
  if (fee.kind === "fixed") return;
  const amountPath = `${feePath}/amountArg`;
  const amount = argSchema(step, fee.amountArg);
  if (amount === undefined) err("unknown-arg", amountPath, `arg '${fee.amountArg}' is not declared`);
  if (amount.type !== "string" || amount.format !== "u64") {
    err("invalid-fee-basis", amountPath, "the amount arg must be a string with format 'u64'");
  }
  if (fee.asset.kind === "spl") {
    const sourcePath = `${feePath}/asset/sourceArg`;
    const source = argSchema(step, fee.asset.sourceArg);
    if (source === undefined) {
      err("unknown-arg", sourcePath, `arg '${fee.asset.sourceArg}' is not declared`);
    }
    if (source.type !== "string" || source.format !== "pubkey") {
      err("invalid-fee-basis", sourcePath, "the source arg must be a string with format 'pubkey'");
    }
  }
}

function checkTransitions(spec: ProtocolSpecV1, step: StepV1, base: string): void {
  if (step.scope === "create") {
    if (step.transitions.length > 0) {
      err(
        "create-step-transition",
        `${base}/transitions/0`,
        "a create step starts the instance in its initial state and declares no transitions",
      );
    }
    return;
  }
  step.transitions.forEach((transition, i) => {
    const path = `${base}/transitions/${i}`;
    if (transition.machine === "participant" && step.scope !== "participant") {
      err("invalid-value", `${path}/machine`, "only participant steps move the participant machine");
    }
    const machine = machineOf(spec, transition.machine)!;
    transition.from.forEach((from, j) => requireState(machine, from, `${path}/from/${j}`));
    requireState(machine, transition.to, `${path}/to`);
    if (machine.states[transition.to]!.kind !== "recorded") {
      err(
        "transition-targets-derived-state",
        `${path}/to`,
        `'${transition.to}' is derived; steps may only move to recorded states`,
      );
    }
  });
}

function checkBind(spec: ProtocolSpecV1, step: StepV1, base: string): void {
  const bindPath = `${base}/bind`;
  if (step.scope === "create") {
    if (step.bind === undefined) err("missing-field", bindPath, "the create step declares bind");
    for (const role of sortedKeys(step.bind.roles)) {
      if (!has(spec.roles, role)) {
        err("unknown-role", `${bindPath}/roles/${role}`, `role '${role}' is not declared`);
      }
    }
    const unbound = sortedKeys(spec.roles).find((role) => !has(step.bind!.roles, role));
    if (unbound !== undefined) {
      err("role-not-bound", `${bindPath}/roles`, `role '${unbound}' must be bound by the create step`);
    }
    return;
  }
  if (step.bind !== undefined) err("bind-outside-create", bindPath, "only the create step binds");
}

function checkStackSurface(spec: ProtocolSpecV1): void {
  const bind = createStep(spec)?.[1].bind;
  const bound = new Set(bind ? Object.keys(bind.accounts) : []);
  for (const [name, account] of entries(spec.stack.instanceAccounts)) {
    const path = `/stack/instanceAccounts/${name}`;
    if (!bound.has(name)) {
      err("unknown-bound-account", path, `'${name}' is not bound by the create step`);
    }
    requireProgramSpec(spec, account.program, `${path}/program`);
  }
  for (const [name, field] of entries(spec.stack.instanceFields)) {
    if (!has(spec.stack.instanceAccounts, field.account)) {
      err(
        "unknown-bound-account",
        `/stack/instanceFields/${name}/account`,
        `'${field.account}' is not a stack instance account`,
      );
    }
  }
  spec.stack.activity.forEach((activity, i) => {
    const path = `/stack/activity/${i}`;
    if (!has(spec.steps, activity.step)) {
      err("unknown-step", `${path}/step`, `step '${activity.step}' is not declared`);
    }
    specProgram(spec, activity.program, `${path}/program`);
  });
}

/** One machine edge with where it came from. */
export interface MachineEdge {
  from: string[];
  to: string;
  via: { step: string } | { derive: number };
}

/**
 * Edges of a machine: step transitions (an empty `from` means every
 * non-terminal state) and derivation rules, in Rust order.
 */
export function machineEdges(spec: ProtocolSpecV1, kind: MachineKind): MachineEdge[] {
  const machine = machineOf(spec, kind)!;
  const nonTerminal = entries(machine.states)
    .filter(([, state]) => !state.terminal)
    .map(([name]) => name);
  const edges: MachineEdge[] = [];
  for (const [name, step] of entries(spec.steps)) {
    for (const transition of step.transitions.filter((t) => t.machine === kind)) {
      edges.push({
        from: transition.from.length === 0 ? [...nonTerminal] : [...transition.from],
        to: transition.to,
        via: { step: name },
      });
    }
  }
  machine.derive.forEach((rule, i) => edges.push({ from: [...rule.from], to: rule.to, via: { derive: i } }));
  return edges;
}

function checkReachability(spec: ProtocolSpecV1, kind: MachineKind): void {
  const machine = machineOf(spec, kind)!;
  const base = `/machines/${kind}`;
  const edges = machineEdges(spec, kind);
  const reachable = new Set([machine.initial]);
  for (;;) {
    const before = reachable.size;
    for (const edge of edges) {
      if (edge.from.some((state) => reachable.has(state))) reachable.add(edge.to);
    }
    if (reachable.size === before) break;
  }
  for (const [name, state] of entries(machine.states)) {
    const path = `${base}/states/${name}`;
    if (!reachable.has(name)) {
      err("unreachable-state", path, `'${name}' cannot be reached from '${machine.initial}'`);
    }
    const exits = edges.some((edge) => edge.from.includes(name) && edge.to !== name);
    if (state.terminal && exits) err("terminal-state-exit", path, `terminal state '${name}' has an exit`);
    if (!state.terminal && !exits) err("dead-end-state", path, `non-terminal state '${name}' has no exit`);
  }
}

function triggerMachine(spec: ProtocolSpecV1, trigger: TriggerV1): MachineKind | undefined {
  if (trigger.kind !== "onConfirmed") return trigger.machine;
  if (!has(spec.steps, trigger.step)) return undefined;
  return spec.steps[trigger.step]!.scope === "participant" ? "participant" : "instance";
}

function checkReactions(spec: ProtocolSpecV1): void {
  const ids = new Set<string>();
  spec.reactions.forEach((reaction, i) => {
    const base = `/reactions/${i}`;
    if (ids.has(reaction.id)) {
      err("duplicate-entry", `${base}/id`, `reaction id '${reaction.id}' is used twice`);
    }
    ids.add(reaction.id);
    const triggerPath = `${base}/trigger`;
    const trigger = reaction.trigger;
    if (trigger.kind === "at" || trigger.kind === "when") {
      const machine = machineOf(spec, trigger.machine)!;
      trigger.in.forEach((state, j) => requireState(machine, state, `${triggerPath}/in/${j}`));
    } else if (trigger.kind === "onTransition") {
      const machine = machineOf(spec, trigger.machine)!;
      trigger.from.forEach((state, j) => requireState(machine, state, `${triggerPath}/from/${j}`));
      trigger.to.forEach((state, j) => requireState(machine, state, `${triggerPath}/to/${j}`));
    } else if (!has(spec.steps, trigger.step)) {
      err("unknown-step", `${triggerPath}/step`, `step '${trigger.step}' is not declared`);
    }
    const actionPath = `${base}/action`;
    const action = reaction.action;
    if (action.kind === "runKeeperStep" || action.kind === "proposeSquads") {
      if (!has(spec.steps, action.step)) {
        err("unknown-step", `${actionPath}/step`, `step '${action.step}' is not declared`);
      }
      const target = spec.steps[action.step]!;
      const matches =
        action.kind === "runKeeperStep"
          ? target.authority.kind === "keeper"
          : target.authority.kind === "squadsProposer";
      if (!matches) {
        err(
          "reaction-action-mismatch",
          `${actionPath}/step`,
          "runKeeperStep targets keeper steps and proposeSquads targets squadsProposer steps",
        );
      }
    } else {
      if (!has(spec.events, action.event)) {
        err("unknown-event", `${actionPath}/event`, `event '${action.event}' is not declared`);
      }
      const declared = spec.events[action.event]!;
      if (declared.machine === "participant" && triggerMachine(spec, trigger) !== "participant") {
        err("event-machine-mismatch", `${actionPath}/event`, "participant events need a participant trigger");
      }
    }
  });
}

function checkEvents(spec: ProtocolSpecV1): void {
  for (const name of sortedKeys(spec.events)) {
    const emitted = spec.reactions.some(
      (reaction) => reaction.action.kind === "emit" && reaction.action.event === name,
    );
    if (!emitted) err("event-not-emitted", `/events/${name}`, `no emit reaction publishes '${name}'`);
  }
}
