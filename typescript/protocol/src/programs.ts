/**
 * Installed programs: the program modules `a4 install program --ts`
 * generates, as a protocol uses them.
 *
 * A generated program (the default export of `@programs/<name>`) carries
 * two plain properties for protocols, besides its `programId` and
 * `programSpecHash`:
 *
 * - `programInstall`: how the project installed it ({@link ProgramInstall}):
 *   the arete.toml alias, the program release when it came from the
 *   registry, and its source (a registry package at the exact installed
 *   version, or a project-relative IDL file).
 * - `instructionShapes`: each instruction's IDL name, argument names and
 *   account names, and which accounts may be left out of a template
 *   ({@link InstructionShape}).
 *
 * This package reads only those properties, so it never imports the SDK
 * runtime. The shape is stable: new fields may be added, existing ones keep
 * their meaning.
 */

import { BUILTIN_PROGRAMS, type BuiltinProgram } from "./contracts/types.js";

/** Where an installed program comes from. */
export type ProgramInstallSource =
  | {
      readonly kind: "registry";
      /** The registry package. */
      readonly registry: string;
      /** The exact installed version (`1.2.3`), as arete.lock pins it. */
      readonly version: string;
    }
  | {
      readonly kind: "idl";
      /** The IDL file, relative to the project root (the arete.toml directory). */
      readonly idl: string;
    };

/** `programInstall` on a generated program: how the project installed it. */
export interface ProgramInstall {
  /** The dependency's alias in arete.toml (`[dependencies.programs.<alias>]`). */
  readonly alias: string;
  /**
   * The installed program release: a registry install's, or, for an IDL
   * install, the one `a4 protocol deploy` recorded after pushing the program.
   */
  readonly programReleaseHash?: string;
  readonly source: ProgramInstallSource;
}

/**
 * One account of an instruction shape.
 *
 * - `optional`: the IDL marks the account optional.
 * - `derive`: `null` when a template must supply the account; otherwise the
 *   accounts its address is derived from (`[]` for a fixed address or a PDA
 *   of literal and argument seeds).
 */
export interface InstructionAccountShape {
  readonly optional: boolean;
  readonly derive: readonly string[] | null;
}

/** One instruction of `instructionShapes`, keyed by its IDL name. */
export interface InstructionShape {
  /** Argument names, in IDL order. */
  readonly args: readonly string[];
  /** Accounts by IDL name, in IDL order. */
  readonly accounts: Readonly<Record<string, InstructionAccountShape>>;
}

/** A generated program module, as a protocol reads it. */
export interface InstalledProgram {
  readonly programId: string;
  readonly programSpecHash: string;
  readonly programInstall: ProgramInstall;
  readonly instructionShapes: Readonly<Record<string, InstructionShape>>;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object";
}

/**
 * Whether `value` is a generated program module: detected by its
 * `programInstall` shape, never by class, so a module from another copy of
 * the SDK (or plain JavaScript) is recognised too.
 */
export function isInstalledProgram(value: unknown): value is InstalledProgram {
  if (!isRecord(value) || !isRecord(value.programInstall) || !isRecord(value.instructionShapes)) return false;
  if (typeof value.programId !== "string" || typeof value.programSpecHash !== "string") return false;
  const install = value.programInstall;
  if (typeof install.alias !== "string" || !isRecord(install.source)) return false;
  if (install.programReleaseHash !== undefined && typeof install.programReleaseHash !== "string") return false;
  const source = install.source;
  if (source.kind === "registry") return typeof source.registry === "string" && typeof source.version === "string";
  if (source.kind === "idl") return typeof source.idl === "string";
  return false;
}

/**
 * Builtin program entries for `programs`, keyed by builtin name:
 * `builtins("system", "splToken")` is
 * `{ system: { builtin: "system" }, splToken: { builtin: "splToken" } }`.
 */
export function builtins<const N extends readonly BuiltinProgram[]>(
  ...names: N
): { readonly [K in N[number]]: { readonly builtin: K } } {
  const out: Record<string, { builtin: BuiltinProgram }> = {};
  for (const name of names) {
    if (!(BUILTIN_PROGRAMS as readonly string[]).includes(name)) {
      throw new TypeError(`'${String(name)}' is not a builtin program (${BUILTIN_PROGRAMS.join(", ")})`);
    }
    out[name] = { builtin: name };
  }
  return out as { readonly [K in N[number]]: { readonly builtin: K } };
}

// ---------------------------------------------------------------------------
// Template checks
// ---------------------------------------------------------------------------

/** Names compare as the server compares them: exactly, else ignoring case and `_`. */
function normalize(name: string): string {
  return name.replace(/_/g, "").toLowerCase();
}

function findName<T>(names: Iterable<[string, T]>, name: string): [string, T] | undefined {
  const entries = [...names];
  return entries.find(([key]) => key === name) ?? entries.find(([key]) => normalize(key) === normalize(name));
}

/** One template problem: a JSON-Pointer-like path under the template and a message. */
export interface TemplateShapeProblem {
  path: string;
  message: string;
}

/**
 * Check a template instruction against an installed program's instruction
 * shapes, as Arete checks it against the program's ProgramSpec:
 *
 * - the instruction exists;
 * - every argument it names exists, and every argument is supplied;
 * - every account it names exists;
 * - every account it leaves out is optional, or derivable: a fixed address,
 *   or a PDA whose seed accounts are each supplied or derivable in turn.
 */
export function checkTemplateShape(
  program: InstalledProgram,
  template: { instruction: string; args: Readonly<Record<string, string>>; accounts: Readonly<Record<string, string>> },
): TemplateShapeProblem | undefined {
  const found = findName(Object.entries(program.instructionShapes), template.instruction);
  if (found === undefined) {
    return {
      path: "instruction",
      message: `program '${program.programInstall.alias}' has no instruction '${template.instruction}'`,
    };
  }
  const [name, shape] = found;
  for (const arg of Object.keys(template.args)) {
    if (findName(shape.args.map((a) => [a, a] as [string, string]), arg) === undefined) {
      return { path: `args/${arg}`, message: `instruction '${name}' has no argument '${arg}'` };
    }
  }
  for (const arg of shape.args) {
    if (findName(Object.entries(template.args), arg) === undefined) {
      return { path: "args", message: `argument '${arg}' of '${name}' is not supplied` };
    }
  }
  for (const account of Object.keys(template.accounts)) {
    if (findName(Object.entries(shape.accounts), account) === undefined) {
      return { path: `accounts/${account}`, message: `instruction '${name}' has no account '${account}'` };
    }
  }
  const supplied = new Set(
    Object.keys(shape.accounts).filter((account) => findName(Object.entries(template.accounts), account) !== undefined),
  );
  const derivable = (account: string, depth: number): boolean => {
    if (depth > 8) return false;
    const derive = shape.accounts[account]?.derive;
    if (derive === null || derive === undefined) return false;
    return derive.every((seed) => supplied.has(seed) || derivable(seed, depth + 1));
  };
  for (const [account, accountShape] of Object.entries(shape.accounts)) {
    if (accountShape.optional || supplied.has(account) || derivable(account, 0)) continue;
    return { path: "accounts", message: `account '${account}' of '${name}' is neither supplied nor derivable` };
  }
  return undefined;
}

// ---------------------------------------------------------------------------
// Typed templates
// ---------------------------------------------------------------------------

type ArgNames<S extends InstructionShape> = S["args"][number] & string;

/** Accounts a template must supply: not optional and not derivable. */
export type RequiredAccounts<S extends InstructionShape> = {
  [K in keyof S["accounts"]]: S["accounts"][K] extends { readonly optional: false; readonly derive: null } ? K : never;
}[keyof S["accounts"]] &
  string;

/** Accounts a template may leave out. */
export type OmittableAccounts<S extends InstructionShape> = Exclude<keyof S["accounts"] & string, RequiredAccounts<S>>;

type AccountsInput<S extends InstructionShape> = { readonly [K in RequiredAccounts<S>]: string } & {
  readonly [K in OmittableAccounts<S>]?: string;
};

/** The `{ args, accounts, remainingAccounts }` of `ix`, typed from an instruction shape. */
export type IxInput<S extends InstructionShape> = ([ArgNames<S>] extends [never]
  ? { readonly args?: Readonly<Record<string, never>> }
  : { readonly args: { readonly [K in ArgNames<S>]: string } }) &
  ([RequiredAccounts<S>] extends [never]
    ? { readonly accounts?: AccountsInput<S> }
    : { readonly accounts: AccountsInput<S> }) & {
    /**
     * Trailing accounts the IDL does not name, appended after its accounts
     * in this order. `pubkey` is an account expression; they never sign.
     */
    readonly remainingAccounts?: readonly { pubkey: string; isWritable?: boolean; isSigner?: false }[];
  };

/** The instruction names of an installed program. */
export type InstructionName<P extends InstalledProgram> = keyof P["instructionShapes"] & string;

/** A template instruction `ix` returns: its program is the module itself. */
export interface ProgramTemplateInstruction<P extends InstalledProgram = InstalledProgram> {
  readonly program: P;
  readonly instruction: string;
  readonly args?: Readonly<Record<string, string>>;
  readonly accounts?: Readonly<Record<string, string>>;
  readonly remainingAccounts?: readonly { pubkey: string; isWritable?: boolean; isSigner?: false }[];
}

/**
 * A template instruction of an installed program, checked at compile time:
 * the instruction name, every argument name and every account name come
 * from the program's IDL (as written there, e.g. `creator_withdraw`), every
 * argument must be given, and every account the program can't derive must
 * be given. Values are expressions, as in any template.
 *
 * An account may be left out when the IDL marks it optional, or when its
 * address is derivable: a fixed address (a program or sysvar), or a PDA of
 * literal and argument seeds whose account seeds are supplied or derivable
 * in turn. The type allows leaving out any derivable account; when the
 * protocol is defined, a left-out PDA whose seed accounts are missing is
 * reported.
 *
 * `program` must be one of the definition's `programs`; its alias there is
 * the template's `program`. `defineProtocol` checks the names again against
 * the program's runtime shapes, which catches plain JavaScript and stale
 * type declarations.
 *
 * ```ts
 * ix(meteoraPresale, "creator_withdraw", {
 *   args: { remaining_accounts_info: "resources.noRemainingAccounts" },
 *   accounts: { presale: "instance.accounts.presale", owner: 'pda("squadsV4", "vault", resources.treasury.multisig, 0)' },
 * })
 * ```
 */
export function ix<P extends InstalledProgram, N extends InstructionName<P>>(
  program: P,
  instruction: N,
  input: IxInput<P["instructionShapes"][N]>,
): ProgramTemplateInstruction<P> {
  const { args, accounts, remainingAccounts } = input as {
    args?: Readonly<Record<string, string>>;
    accounts?: Readonly<Record<string, string>>;
    remainingAccounts?: ProgramTemplateInstruction["remainingAccounts"];
  };
  return {
    program,
    instruction,
    ...(args !== undefined ? { args } : {}),
    ...(accounts !== undefined ? { accounts } : {}),
    ...(remainingAccounts !== undefined ? { remainingAccounts } : {}),
  };
}
