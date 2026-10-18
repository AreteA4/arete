/**
 * `defineProtocol`: the TypeScript authoring front end.
 *
 * A definition is plain data plus handler functions. `lowerProtocol` lowers
 * it to the canonical `arete.protocol-spec/v1` document: it fills every
 * default the contract requires, so two definitions that mean the same
 * thing produce the same bytes and hash, and it collects handler functions
 * for bundling. The spec it returns has passed `validateProtocolSpec`.
 *
 * `defineProtocol` itself only brands its argument; keep it that way so a
 * handler bundle that imports the definition pulls in no lowering code.
 */

import type { ProtocolHandler } from "./abi.js";
import { validateProtocolSpec } from "./contracts/spec.js";
import { compareUtf8 } from "./contracts/shape.js";
import { checkTemplateShape, isInstalledProgram, type InstalledProgram, type ProgramInstall } from "./programs.js";
import type { StackProgramSource } from "./stack/generate.js";
import {
  PROTOCOL_CTX_V1,
  PROTOCOL_HANDLER_ABI_V1,
  PROTOCOL_SPEC_SCHEMA_V1,
  STRING_FORMATS,
  isClientCallable,
  type ActionV1,
  type AuthorityV1,
  type BuiltinProgram,
  type DeriveRuleV1,
  type EventV1,
  type FeeBasisV1,
  type FieldSchemaV1,
  type GlueV1,
  type GuardV1,
  type InstanceVisibility,
  type MachineKind,
  type MachineV1,
  type MetadataV1,
  type ObjectSchemaV1,
  type ProtocolProgramV1,
  type ProtocolSpecV1,
  type ProtocolStackDependencyV1,
  type ReactionV1,
  type ReadV1,
  type ResourceDeclV1,
  type RoleV1,
  type StackSurfaceV1,
  type StateKind,
  type StateV1,
  type StepAccess,
  type StepScope,
  type StepV1,
  type StringFormat,
  type TemplateInstructionV1,
  type TransitionV1,
  type TriggerV1,
} from "./contracts/types.js";

// ---------------------------------------------------------------------------
// Authoring types
// ---------------------------------------------------------------------------

/** A string format (`"u64"`, `"pubkey"`, ...) or a bare JSON type. */
export type FieldShorthand = StringFormat | "string" | "boolean" | "integer";
export type FieldInput = FieldSchemaV1 | FieldShorthand;

/**
 * A full object schema, or a map of property name → field. In the map form
 * every property is required and `required` is sorted.
 */
export type ObjectSchemaInput = ObjectSchemaV1 | Readonly<Record<string, FieldInput>>;

/**
 * A program the protocol uses: a program module installed with `a4 install
 * program --ts` (`import vault from "@programs/vault"`), or a builtin
 * (`builtins(...)`). A module pins the exact ProgramSpec it was generated
 * from, and the release when it came from the registry.
 */
export type ProgramInput =
  | InstalledProgram
  | ProtocolProgramV1
  | { programId: string; programSpecHash: string; programReleaseHash?: string }
  | { builtin: BuiltinProgram };

export type ReadInput =
  | ReadV1
  | { kind: "account"; program: string; account: string; address: string; optional?: boolean }
  | { kind: "tokenAccount"; address: string; optional?: boolean };

export type StateInput = StateKind | { kind?: StateKind; terminal?: boolean; description?: string };

export interface MachineInput {
  initial: string;
  states: Readonly<Record<string, StateInput>>;
  derive?: readonly DeriveRuleV1[];
  reads?: readonly string[];
}

/** Guards as `{ code: expr }` (or `{ code: { expr, message } }`), or the contract's list form. */
export type GuardsInput =
  | readonly GuardV1[]
  | Readonly<Record<string, string | { expr: string; message?: string }>>;

export interface TemplateInstructionInput {
  /** A program alias of the definition's `programs`, or the installed program module itself (`ix` passes the module). */
  program: string | InstalledProgram;
  /** The instruction name. `ix` is an alias. */
  instruction?: string;
  ix?: string;
  args?: Readonly<Record<string, string>>;
  accounts?: Readonly<Record<string, string>>;
  /**
   * Trailing accounts the IDL does not name, appended after its accounts in
   * this order (e.g. accounts a program reads from `remaining_accounts`).
   * `pubkey` is an account expression; remaining accounts never sign.
   */
  remainingAccounts?: readonly RemainingAccountInput[];
}

export interface RemainingAccountInput {
  pubkey: string;
  /** Defaults to false. */
  isWritable?: boolean;
  /** Remaining accounts never sign; only `false` is accepted. */
  isSigner?: false;
}

export interface TransitionInput {
  /** Defaults to `instance`. */
  machine?: MachineKind;
  /** Empty (the default) means every non-terminal state. */
  from?: readonly string[];
  to: string;
}

export type AuthorityInput = AuthorityV1 | "creator" | "participant" | "keeper";

interface StepInputBase {
  scope: StepScope;
  title?: string;
  description?: string;
  authority: AuthorityInput;
  /** Defaults to `secret` on create steps and `public` on other wallet steps. */
  access?: StepAccess;
  args?: ObjectSchemaInput;
  guards?: GuardsInput;
  reads?: readonly string[];
  ephemeral?: readonly string[];
  fee?: FeeBasisV1;
  transitions?: readonly TransitionInput[];
  bind?: { accounts?: Readonly<Record<string, string>>; roles?: Readonly<Record<string, string>> };
  glue?: Partial<GlueV1>;
}

export interface TemplateStepInput extends StepInputBase {
  build: readonly TemplateInstructionInput[];
  handler?: never;
}

export interface HandlerStepInput extends StepInputBase {
  /** The reviewed escape hatch: a handler bundled into the release. */
  handler: ProtocolHandler<any, any>;
  /** Program aliases the handler's output may use (`build.programs`). */
  programs: readonly string[];
  /** The bundle export name; defaults to the step name. */
  export?: string;
  build?: never;
}

export type StepInput = TemplateStepInput | HandlerStepInput;

export interface ProtocolDefinition {
  name: string;
  version: string;
  description?: string;
  programs?: Readonly<Record<string, ProgramInput>>;
  stacks?: Readonly<Record<string, { stackManifestHash: string; package?: string }>>;
  params?: ObjectSchemaInput;
  roles?: Readonly<Record<string, RoleV1>>;
  resources?: Readonly<Record<string, ResourceDeclV1 | { schema: FieldInput; description?: string }>>;
  instanceVisibility: InstanceVisibility;
  reads?: Readonly<Record<string, ReadInput>>;
  machines: { instance: MachineInput; participant?: MachineInput };
  steps: Readonly<Record<string, StepInput>>;
  reactions?: readonly ReactionV1[];
  events?: Readonly<Record<string, EventV1 | MachineKind>>;
  metadata?: MetadataV1;
  stack?: Partial<StackSurfaceV1>;
}

const DEFINED = "__areteProtocolDefinition";

export type DefinedProtocol<D extends ProtocolDefinition = ProtocolDefinition> = D & {
  readonly [DEFINED]: true;
};

/** Step names of a definition. */
export type StepName<D extends ProtocolDefinition> = keyof D["steps"] & string;

type GuardCodes<G> = G extends readonly { code: infer C }[]
  ? C & string
  : G extends Readonly<Record<string, unknown>>
    ? keyof G & string
    : never;

/** The typed rejection codes of a step: its guard codes. */
export type RejectionCode<D extends ProtocolDefinition, S extends StepName<D>> = GuardCodes<
  D["steps"][S]["guards"]
>;

/** Event names a definition declares. */
export type EventName<D extends ProtocolDefinition> = keyof NonNullable<D["events"]> & string;

/**
 * Declare a protocol. The result is the definition itself, branded; lower it
 * with `lowerProtocol` (the `arete-protocol` CLI does this for you).
 */
export function defineProtocol<const D extends ProtocolDefinition>(definition: D): DefinedProtocol<D> {
  Object.defineProperty(definition, DEFINED, { value: true, enumerable: false });
  return definition as DefinedProtocol<D>;
}

export function isDefinedProtocol(value: unknown): value is DefinedProtocol {
  return value !== null && typeof value === "object" && (value as Record<string, unknown>)[DEFINED] === true;
}

// ---------------------------------------------------------------------------
// Lowering
// ---------------------------------------------------------------------------

export interface LoweredProtocol {
  spec: ProtocolSpecV1;
  /** Handler functions by bundle export name. */
  handlers: Record<string, ProtocolHandler<any, any>>;
  /** Step name → bundle export name, for steps with handlers. */
  handlerExports: Record<string, string>;
  /**
   * Where the generated stack reads each installed program's IDL, by alias:
   * a registry package at the exact installed version, or the IDL file the
   * build copies to `idl/<file>` beside the stack source. Programs given by
   * raw hash are absent and get the generator's default.
   */
  programSources: Record<string, StackProgramSource>;
  /** The installed program modules by alias, for the build's arete.lock check and IDL copies. */
  installedPrograms: Record<string, InstalledProgramIdentity>;
}

/** What the build needs of an installed program module. */
export interface InstalledProgramIdentity {
  programId: string;
  programSpecHash: string;
  programInstall: ProgramInstall;
}

export class ProtocolDefinitionError extends Error {
  readonly path: string;

  constructor(path: string, message: string) {
    super(`${path}: ${message}`);
    this.name = "ProtocolDefinitionError";
    this.path = path;
  }
}

function definitionError(path: string, message: string): never {
  throw new ProtocolDefinitionError(path, message);
}

function mapValues<T, U>(record: Readonly<Record<string, T>> | undefined, map: (value: T, key: string) => U): Record<string, U> {
  const out: Record<string, U> = {};
  for (const [key, value] of Object.entries(record ?? {})) out[key] = map(value, key);
  return out;
}

function lowerField(field: FieldInput): FieldSchemaV1 {
  if (typeof field !== "string") return field;
  if ((STRING_FORMATS as readonly string[]).includes(field)) {
    return { type: "string", format: field as StringFormat };
  }
  return { type: field as "string" | "boolean" | "integer" };
}

function lowerObjectSchema(schema: ObjectSchemaInput | undefined): ObjectSchemaV1 {
  if (schema === undefined) return { type: "object", properties: {}, required: [], additionalProperties: false };
  if ((schema as ObjectSchemaV1).type === "object" && typeof (schema as ObjectSchemaV1).properties === "object") {
    return schema as ObjectSchemaV1;
  }
  const properties = mapValues(schema as Record<string, FieldInput>, lowerField);
  return {
    type: "object",
    properties,
    required: Object.keys(properties).sort(compareUtf8),
    additionalProperties: false,
  };
}

function lowerProgram(program: ProgramInput): ProtocolProgramV1 {
  if (isInstalledProgram(program)) {
    const release = program.programInstall.programReleaseHash;
    return {
      kind: "programSpec",
      programId: program.programId,
      programSpecHash: program.programSpecHash as never,
      ...(release !== undefined ? { programReleaseHash: release as never } : {}),
    };
  }
  if ("kind" in program) return program;
  if ("builtin" in program) return { kind: "builtin", builtin: program.builtin };
  return {
    kind: "programSpec",
    programId: program.programId,
    programSpecHash: program.programSpecHash as never,
    ...(program.programReleaseHash !== undefined ? { programReleaseHash: program.programReleaseHash as never } : {}),
  };
}

function lowerRead(read: ReadInput): ReadV1 {
  switch (read.kind) {
    case "account":
      return {
        kind: "account",
        program: read.program,
        account: read.account,
        address: read.address,
        optional: read.optional ?? false,
      };
    case "tokenAccount":
      return { kind: "tokenAccount", address: read.address, optional: read.optional ?? false };
    default:
      return read;
  }
}

function lowerState(state: StateInput): StateV1 {
  if (typeof state === "string") return { kind: state, terminal: false };
  return {
    kind: state.kind ?? "recorded",
    terminal: state.terminal ?? false,
    ...(state.description !== undefined ? { description: state.description } : {}),
  };
}

function lowerMachine(machine: MachineInput): MachineV1 {
  return {
    initial: machine.initial,
    states: mapValues(machine.states, lowerState),
    derive: (machine.derive ?? []).map((rule) => ({
      from: [...rule.from],
      to: rule.to,
      when: rule.when,
      ...(rule.at !== undefined ? { at: rule.at } : {}),
    })),
    reads: [...(machine.reads ?? [])],
  };
}

function lowerAuthority(authority: AuthorityInput): AuthorityV1 {
  return typeof authority === "string" ? ({ kind: authority } as AuthorityV1) : authority;
}

function lowerGuards(guards: GuardsInput | undefined): GuardV1[] {
  if (guards === undefined) return [];
  if (Array.isArray(guards)) {
    return (guards as readonly GuardV1[]).map((guard) => ({
      code: guard.code,
      expr: guard.expr,
      ...(guard.message !== undefined ? { message: guard.message } : {}),
    }));
  }
  return Object.entries(guards as Record<string, string | { expr: string; message?: string }>).map(
    ([code, guard]) =>
      typeof guard === "string"
        ? { code, expr: guard }
        : { code, expr: guard.expr, ...(guard.message !== undefined ? { message: guard.message } : {}) },
  );
}

/** The definition's programs: installed modules by identity, and aliases. */
interface ProgramTable {
  aliases: Map<InstalledProgram, string>;
  modules: Map<string, InstalledProgram>;
}

function programTable(programs: ProtocolDefinition["programs"]): ProgramTable {
  const table: ProgramTable = { aliases: new Map(), modules: new Map() };
  for (const [alias, program] of Object.entries(programs ?? {})) {
    if (!isInstalledProgram(program)) continue;
    if (table.aliases.has(program)) {
      definitionError(`/programs/${alias}`, `the same program module is also declared as '${table.aliases.get(program)}'`);
    }
    table.aliases.set(program, alias);
    table.modules.set(alias, program);
  }
  return table;
}

function lowerTemplate(instruction: TemplateInstructionInput, path: string, programs: ProgramTable): TemplateInstructionV1 {
  const name = instruction.instruction ?? instruction.ix;
  if (instruction.instruction !== undefined && instruction.ix !== undefined && instruction.instruction !== instruction.ix) {
    definitionError(path, "give either 'instruction' or its alias 'ix', not both");
  }
  if (name === undefined) definitionError(path, "a template instruction names its 'instruction' (or 'ix')");
  let program: string;
  if (typeof instruction.program === "string") {
    program = instruction.program;
  } else {
    const alias = programs.aliases.get(instruction.program);
    if (alias === undefined) {
      const install = isInstalledProgram(instruction.program) ? ` ('${instruction.program.programInstall.alias}')` : "";
      definitionError(`${path}/program`, `the program${install} is not declared in 'programs'`);
    }
    program = alias;
  }
  const module = programs.modules.get(program);
  if (module !== undefined) {
    const problem = checkTemplateShape(module, {
      instruction: name,
      args: instruction.args ?? {},
      accounts: instruction.accounts ?? {},
    });
    if (problem !== undefined) definitionError(`${path}/${problem.path}`, problem.message);
  }
  const remaining = (instruction.remainingAccounts ?? []).map((account, i) => {
    if ((account.isSigner as boolean | undefined) === true) {
      definitionError(`${path}/remainingAccounts/${i}`, "remaining accounts cannot sign; name signers as IDL accounts");
    }
    return { pubkey: account.pubkey, isWritable: account.isWritable ?? false, isSigner: false as const };
  });
  return {
    program,
    instruction: name,
    args: { ...(instruction.args ?? {}) },
    accounts: { ...(instruction.accounts ?? {}) },
    ...(remaining.length > 0 ? { remainingAccounts: remaining } : {}),
  };
}

function lowerTransition(transition: TransitionInput): TransitionV1 {
  return { machine: transition.machine ?? "instance", from: [...(transition.from ?? [])], to: transition.to };
}

function lowerStep(name: string, step: StepInput, handlers: LoweredProtocol, programs: ProgramTable): StepV1 {
  const path = `/steps/${name}`;
  const authority = lowerAuthority(step.authority);
  // Create and role steps default to `secret`: instances are created and roles
  // acted on from a backend. Other wallet steps default to `public`.
  const access: StepAccess | undefined =
    step.access ??
    (isClientCallable(authority)
      ? step.scope === "create" || authority.kind === "role"
        ? "secret"
        : "public"
      : undefined);
  const glue: GlueV1 = {
    atas: (step.glue?.atas ?? []).map((ata) => ({ owner: ata.owner, mint: ata.mint })),
    ...(step.glue?.wrapSol !== undefined ? { wrapSol: { amount: step.glue.wrapSol.amount } } : {}),
    unwrapSol: step.glue?.unwrapSol ?? false,
  };
  let build: StepV1["build"];
  if ("handler" in step && step.handler !== undefined) {
    if ((step as { build?: unknown }).build !== undefined) {
      definitionError(path, "a step has exactly one of a template ('build') or a 'handler'");
    }
    const exportName = step.export ?? name;
    if (Object.prototype.hasOwnProperty.call(handlers.handlers, exportName)) {
      definitionError(path, `handler export '${exportName}' is used by two steps`);
    }
    handlers.handlers[exportName] = step.handler;
    handlers.handlerExports[name] = exportName;
    build = { kind: "handler", export: exportName, programs: [...step.programs] };
  } else {
    const template = (step as TemplateStepInput).build;
    if (!Array.isArray(template)) definitionError(path, "a step needs a template ('build') or a 'handler'");
    build = {
      kind: "template",
      instructions: template.map((instruction, i) => lowerTemplate(instruction, `${path}/build/${i}`, programs)),
    };
  }
  const out: StepV1 = {
    scope: step.scope,
    ...(step.title !== undefined ? { title: step.title } : {}),
    ...(step.description !== undefined ? { description: step.description } : {}),
    authority,
    ...(access !== undefined ? { access } : {}),
    args: lowerObjectSchema(step.args),
    guards: lowerGuards(step.guards),
    reads: [...(step.reads ?? [])],
    ephemeral: [...(step.ephemeral ?? [])],
    fee: step.fee ?? { kind: "none" },
    transitions: (step.transitions ?? []).map(lowerTransition),
    glue,
    build,
  };
  if (step.scope === "create" || step.bind !== undefined) {
    out.bind = { accounts: { ...(step.bind?.accounts ?? {}) }, roles: { ...(step.bind?.roles ?? {}) } };
  }
  return out;
}

function lowerTrigger(trigger: TriggerV1): TriggerV1 {
  switch (trigger.kind) {
    case "at":
      return { kind: "at", machine: trigger.machine, in: [...trigger.in], time: trigger.time };
    case "when":
      return { kind: "when", machine: trigger.machine, in: [...trigger.in], condition: trigger.condition };
    case "onTransition":
      return { kind: "onTransition", machine: trigger.machine, from: [...trigger.from], to: [...trigger.to] };
    case "onConfirmed":
      return { kind: "onConfirmed", step: trigger.step };
  }
}

function lowerAction(action: ActionV1): ActionV1 {
  return action.kind === "emit" ? { kind: "emit", event: action.event } : { kind: action.kind, step: action.step };
}

/** The file name an installed IDL takes beside the stack source. */
export function idlFileName(idl: string): string {
  return idl.split(/[\\/]/).pop()!;
}

/**
 * Lower a definition to its canonical spec and collect its handlers.
 * Throws `ProtocolDefinitionError` for authoring mistakes and
 * `ProtocolContractError` when the lowered spec is invalid.
 */
export function lowerProtocol(definition: ProtocolDefinition): LoweredProtocol {
  if ((definition as { stackSource?: unknown }).stackSource !== undefined) {
    definitionError(
      "/stackSource",
      "'stackSource' was removed: install each program (`a4 install program`) and import it; the stack reads the program from where it was installed",
    );
  }
  const programs = programTable(definition.programs);
  const lowered: LoweredProtocol = {
    spec: undefined as never,
    handlers: {},
    handlerExports: {},
    programSources: {},
    installedPrograms: {},
  };
  const idlFiles = new Map<string, string>();
  for (const [alias, module] of programs.modules) {
    const { programInstall } = module;
    if (programInstall.source.kind === "idl") {
      const file = idlFileName(programInstall.source.idl);
      const other = idlFiles.get(file);
      if (other !== undefined && other !== programInstall.source.idl) {
        definitionError(`/programs/${alias}`, `IDLs '${other}' and '${programInstall.source.idl}' share the file name '${file}'; rename one`);
      }
      idlFiles.set(file, programInstall.source.idl);
    }
    lowered.installedPrograms[alias] = {
      programId: module.programId,
      programSpecHash: module.programSpecHash,
      programInstall,
    };
    lowered.programSources[alias] =
      programInstall.source.kind === "registry"
        ? { registry: programInstall.source.registry, version: `=${programInstall.source.version}` }
        : { idl: `./idl/${idlFileName(programInstall.source.idl)}` };
  }
  const steps = mapValues(definition.steps, (step, name) => lowerStep(name, step, lowered, programs));
  const hasHandler = Object.values(steps).some((step) => step.build.kind === "handler");
  const spec: ProtocolSpecV1 = {
    schema: PROTOCOL_SPEC_SCHEMA_V1,
    name: definition.name,
    version: definition.version,
    ...(definition.description !== undefined ? { description: definition.description } : {}),
    ...(hasHandler ? { handlerAbi: PROTOCOL_HANDLER_ABI_V1, ctxVersion: PROTOCOL_CTX_V1 } : {}),
    programs: mapValues(definition.programs, lowerProgram),
    stacks: mapValues(definition.stacks, (stack): ProtocolStackDependencyV1 => ({
      stackManifestHash: stack.stackManifestHash as ProtocolStackDependencyV1["stackManifestHash"],
      ...(stack.package !== undefined ? { package: stack.package } : {}),
    })),
    params: lowerObjectSchema(definition.params),
    roles: mapValues(definition.roles, (role) => (role.description !== undefined ? { description: role.description } : {})),
    resources: mapValues(definition.resources, (resource) => ({
      schema: lowerField(resource.schema),
      ...(resource.description !== undefined ? { description: resource.description } : {}),
    })),
    instanceVisibility: definition.instanceVisibility,
    reads: mapValues(definition.reads, lowerRead),
    machines: {
      instance: lowerMachine(definition.machines.instance),
      ...(definition.machines.participant !== undefined
        ? { participant: lowerMachine(definition.machines.participant) }
        : {}),
    },
    steps,
    reactions: (definition.reactions ?? []).map((reaction) => ({
      id: reaction.id,
      trigger: lowerTrigger(reaction.trigger),
      action: lowerAction(reaction.action),
    })),
    events: mapValues(definition.events, (event) =>
      typeof event === "string"
        ? { machine: event }
        : { machine: event.machine, ...(event.description !== undefined ? { description: event.description } : {}) },
    ),
    metadata: definition.metadata ?? {},
    stack: {
      instanceAccounts: { ...(definition.stack?.instanceAccounts ?? {}) },
      instanceFields: { ...(definition.stack?.instanceFields ?? {}) },
      activity: (definition.stack?.activity ?? []).map((activity) => ({ ...activity })),
    },
  };
  lowered.spec = validateProtocolSpec(JSON.parse(JSON.stringify(spec)) as unknown);
  return lowered;
}
