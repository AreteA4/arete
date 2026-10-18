/**
 * The generated protocol stack.
 *
 * `generateStackSource` turns a spec's `stack` surface into an
 * `arete.stack-source/v1` document (the public Stack Source format) wrapped
 * in `arete.protocol-stack-source/v1`. It has two profiles.
 *
 * **`interim`** reads only the underlying programs: it decodes no memo,
 * labels no `via`, and leaves row scoping to the host. It has three entities:
 *
 * - `ProtocolActivity`: one row per protocol instruction occurrence, keyed
 *   by `id.occurrence` (`<__signature>:<__ix_path>`), with the signature,
 *   instruction path, step, instance account, wallet, amount, slot and time;
 * - `ProtocolInstance`: keyed by the address of the first declared
 *   `stack.instanceAccounts` entry (the primary bound account), with the
 *   `stack.instanceFields` read from that account and per-step activity
 *   counts and sums;
 * - `ProtocolParticipant`: keyed by `id.key` (`<instance account>:<wallet>`),
 *   with per-step counts and sums of each activity's `amountArg`.
 *
 * Keys are single fields because generated SDKs support no composite keys.
 *
 * **Shared instructions.** Several steps may list the same instruction (two
 * steps that both pay through `system.transfer`, say). Every rule that names
 * one instruction must agree on `instanceAccount`, `walletAccount` and
 * `amountArg`, because an occurrence has one of each. Every instruction
 * occurrence is counted once:
 *
 * - without the memo nothing says which of those steps an occurrence
 *   belongs to, so steps that share an instruction (transitively) form one
 *   group named by their sorted step names joined with `_or_`
 *   (`close_or_open`), and per-step fields read from instructions (`step`
 *   flags, `<step>_count`, `<step>_amount`, `<step>_last_slot`, and the
 *   instance's total and direct counts in the full profile) are per group.
 *   A step that shares nothing is a group of its own, under its own name;
 * - in the full profile the attested memo names the step, so the counts
 *   and amounts of attested activity stay per step: a participant's
 *   `<step>_amount` and an instance's `<step>_protocol_amount` only read
 *   instructions of a transaction whose attested memo names that step
 *   (joined through `last_<step>_signature`).
 *
 * **`full`** decodes the attested Memo v2
 * (`arete:p2:<deployment>:<scope|->:<instance>:<participant|->:<step>:<n>/<count>`)
 * through the curated SPL Memo program (an untagged instruction whose only
 * argument is the remaining UTF-8 text). A memo counts only when it carries
 * the protocol prefix and its first account is the deployment's attestation
 * key, listed as a signer. The attestation key, the deployment short id and
 * the program ids are inlined as literals, so each distinct value gives a
 * distinct, deterministic artifact. It sets `lookups: strict`, so each
 * `via` join resolves through the index it names only (a per-step signature
 * join never reaches a row through another step's index). Entities:
 *
 * - `ProtocolInstance`: keyed by the memo's instance id. Per step it counts
 *   the attested activity (`<step>_protocol_count`) and, for a step with an
 *   `amountArg`, sums its amount (`<step>_protocol_amount`; an amount counts
 *   only under the step the transaction's memo names). When the activity's
 *   instance account is one of the instance's own accounts (see **Direct
 *   activity** below), an attested memo also registers that account
 *   (`register_from` with `sibling`), so later activity on it reaches the row
 *   whether or not it carries a memo: the row then counts all activity
 *   (`<group>_count`, per group), the unattested rest
 *   (`<group>_direct_count`) and the amount of all activity
 *   (`<group>_amount`), and `last_signature`, `last_ix_path`, `last_slot` and
 *   `last_timestamp` describe the last activity on those accounts, protocol
 *   or direct. Otherwise the row reads attested memos only, and `last_*`
 *   describe the last attested protocol transaction (the memo instruction);
 * - `ProtocolActivity`: one row per attested protocol transaction, keyed by
 *   its signature, with the memo's instance and participant ids and step,
 *   `attested: true`, `via: "protocol"`, and the wallet, the activity account
 *   (`activity_account`: the account the step's activity entry names as its
 *   `instanceAccount`) and the amount of the activity instructions in that
 *   transaction, each counted once however many steps list it. `instance` is
 *   the memo's short instance id (base58 of the instance UUID's 16 bytes);
 *   the expression language cannot format a UUID, so clients derive the
 *   UUID from it;
 * - `ProtocolParticipant`: keyed by the memo's participant id, with the
 *   instance, the wallet and per-step counts and sums of attested activity
 *   (an amount counts only under the step the transaction's memo names).
 *
 * **Direct activity** (full profile) is activity on the protocol's programs
 * that matches a step's instruction but was not sent through Arete, so it
 * carries no attested memo. Nothing in such an instruction names an
 * instance, so it is matched through the account named by the step's
 * `instanceAccount`, and only when that account belongs to one instance
 * alone: an account the create step binds (`bind.accounts`, under the same
 * name) from one of its ephemeral keys, directly (`ephemeral.vault`) or
 * through a derivation (`pda("p", "presale", ephemeral.base, ...)`). Such an
 * account is fresh for each instance, so it has no history before the
 * instance and no other instance shares it. Any other account (the signing
 * wallet, a role, an `args` or `params` account, an instruction account the
 * create step does not bind) can transact with anyone, or be shared by many
 * instances, so the generator does not match direct activity on it: a step
 * whose instructions name such an account (a bare transfer between two
 * wallets, say) is counted only through attested memos.
 *
 * For `account`-visibility protocols every full-profile entity declares a
 * row `scope` taken from the attested memo's scope id. Direct activity is
 * only ever counted on the instance row (see the known gaps), so it carries
 * the instance row's scope and is visible to exactly the subscribers of
 * that instance. `shared` protocols declare none.
 *
 * Generation is deterministic: the output depends only on the spec and the
 * options, and every collection is emitted in sorted order.
 *
 * The output is a Stack Source document meant to be compiled into the
 * protocol's stack; the compiler's checks are authoritative. Known gaps:
 *
 * - Stack Source cannot pin a program by ProgramSpec hash. Programs are
 *   declared from `options.programs` as a registry slug with a version
 *   requirement, or as a local IDL file (inlining the spec's program id);
 *   without one the slug defaults to the kebab-cased alias and `version` is
 *   omitted, which the compiler rejects until the source is pinned.
 * - Activity routes to the instance through the instruction account named
 *   by `instanceAccount`; the generator assumes it holds the primary bound
 *   account's address.
 * - `instanceFields` read from a non-primary bound account need a
 *   registration neither profile emits, so they are skipped.
 * - Views cannot take a parameter, so `ProtocolActivity/byInstance` and
 *   `ProtocolParticipant/byInstance` are sorted collections; clients filter
 *   by instance. View ids keep the `<Entity>/<name>` form, because the
 *   compiler can only publish those as SDK members.
 * - Full profile: a row per direct (unattested) instruction cannot be
 *   expressed, because an instruction's fields cannot read the instance id
 *   its account is registered to. Direct activity on the instance's own
 *   accounts is therefore counted on the instance row (`<group>_count`,
 *   `<group>_direct_count`, `<group>_amount`, `last_*`), not listed in
 *   `ProtocolActivity`, which holds attested protocol transactions only.
 */

import { byteLength, compareUtf8, decodeBase58, isPubkey, sortedKeys } from "../contracts/shape.js";
import {
  PROTOCOL_STACK_SOURCE_SCHEMA_V1,
  STACK_SOURCE_SCHEMA_V1,
  programIdOf,
  type ProtocolSpecV1,
  type ProtocolStackSourceV1,
  type StackActivityV1,
  type StackProfile,
} from "../contracts/types.js";
import { PACKAGE_NAME, PACKAGE_VERSION } from "../version.js";

export const STACK_GENERATOR = { name: PACKAGE_NAME, version: PACKAGE_VERSION } as const;

/** The memo prefix of Memo v2. */
export const MEMO_V2_PREFIX = "arete:p2:";

/** Where a program's IDL comes from in the generated Stack Source. */
export type StackProgramSource =
  | {
      /** The public registry slug. */
      registry: string;
      /** An exact (`=1.2.3`) or range (`^1.2`, `~1.2`) requirement. */
      version: string;
    }
  | {
      /** A local IDL file, relative to the Stack Source file (`./idl/x.json`). */
      idl: string;
    };

export interface GenerateStackSourceOptions {
  /** `interim` (default) or `full`. */
  profile?: StackProfile;
  /** Program sources by alias. */
  programs?: Readonly<Record<string, StackProgramSource>>;
  /** Full profile: the deployment revision's attestation public key (base58). */
  attestation?: string;
  /** Full profile: the deployment's short id; memos of other deployments are ignored. */
  deploymentShortId?: string;
  /** Full profile: the SPL Memo program source. Defaults to `{ idl: "./idl/spl-memo.idl.json" }`. */
  memoProgram?: StackProgramSource;
  /** The identity recorded in the output. Defaults to `STACK_GENERATOR`. */
  generator?: { name: string; version: string };
}

/** The program alias the full profile gives the SPL Memo program. */
export const MEMO_PROGRAM_ALIAS = "areteMemo";
export const DEFAULT_MEMO_PROGRAM: StackProgramSource = { idl: "./idl/spl-memo.idl.json" };
const MEMO_PROGRAM_ID = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";

export class StackGenerationError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "StackGenerationError";
  }
}

type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

function kebab(text: string): string {
  return text
    .replace(/([a-z0-9])([A-Z])/g, "$1-$2")
    .replace(/_/g, "-")
    .toLowerCase();
}

function pascal(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** Stack Source rejects names equal once non-alphanumerics are removed and case folded. */
function assertNoCollisions(names: string[], where: string): void {
  const seen = new Map<string, string>();
  for (const name of names) {
    const folded = name.replace(/[^A-Za-z0-9]/g, "").toLowerCase();
    const other = seen.get(folded);
    if (other !== undefined && other !== name) {
      throw new StackGenerationError(`${where}: generated names '${other}' and '${name}' collide`);
    }
    seen.set(folded, name);
  }
}

interface StepActivity {
  step: string;
  sources: string[];
  instanceAccount: string;
  walletAccount: string;
  amountArg?: string;
}

/**
 * The steps whose instructions overlap, read as one unit. Without the memo
 * an instruction occurrence cannot say which of the steps that list it it
 * belongs to, so every instruction is counted once, under its group.
 */
interface ActivityGroup extends StepActivity {
  /** The step name, or the sorted step names joined by `_or_`. */
  step: string;
  /** The steps of the group, sorted. */
  steps: StepActivity[];
}

/** The separator of a merged group's step names. */
const GROUP_SEPARATOR = "_or_";

function groupActivity(activity: readonly StackActivityV1[]): StepActivity[] {
  const steps = new Map<string, StepActivity>();
  const owners = new Map<string, StepActivity>();
  for (const entry of activity) {
    const source = `${entry.program}.instructions.${entry.instruction}`;
    const existing = steps.get(entry.step);
    if (existing === undefined) {
      steps.set(entry.step, {
        step: entry.step,
        sources: [source],
        instanceAccount: entry.instanceAccount,
        walletAccount: entry.walletAccount,
        ...(entry.amountArg !== undefined ? { amountArg: entry.amountArg } : {}),
      });
    } else {
      if (
        existing.instanceAccount !== entry.instanceAccount ||
        existing.walletAccount !== entry.walletAccount ||
        existing.amountArg !== entry.amountArg
      ) {
        throw new StackGenerationError(
          `stack.activity for step '${entry.step}': every instruction of a step must name the same instanceAccount, walletAccount and amountArg`,
        );
      }
      if (existing.sources.includes(source)) {
        throw new StackGenerationError(`stack.activity lists ${source} twice for step '${entry.step}'`);
      }
      existing.sources.push(source);
    }
    // One instruction occurrence has one instance account, wallet and amount,
    // whichever step it is attributed to.
    const owner = owners.get(source);
    if (owner === undefined) {
      owners.set(source, steps.get(entry.step)!);
    } else if (
      owner.instanceAccount !== entry.instanceAccount ||
      owner.walletAccount !== entry.walletAccount ||
      owner.amountArg !== entry.amountArg
    ) {
      throw new StackGenerationError(
        `stack.activity: steps '${owner.step}' and '${entry.step}' both list ${source}, so they must name the same instanceAccount, walletAccount and amountArg`,
      );
    }
  }
  return [...steps.values()]
    .map((group) => ({ ...group, sources: [...group.sources].sort(compareUtf8) }))
    .sort((a, b) => compareUtf8(a.step, b.step));
}

/**
 * Merge steps that share an instruction into one group (transitively), so
 * each instruction belongs to exactly one group. A step that shares nothing
 * is a group of its own, named by the step.
 */
function mergeSharedSteps(steps: StepActivity[]): ActivityGroup[] {
  let groups: StepActivity[][] = [];
  for (const step of steps) {
    const overlapping = groups.filter((group) =>
      group.some((other) => other.sources.some((source) => step.sources.includes(source))),
    );
    groups = [...groups.filter((group) => !overlapping.includes(group)), [...overlapping.flat(), step]];
  }
  const merged = groups
    .map((members): ActivityGroup => {
      const sorted = [...members].sort((a, b) => compareUtf8(a.step, b.step));
      const first = sorted[0]!;
      return {
        step: sorted.map((member) => member.step).join(GROUP_SEPARATOR),
        steps: sorted,
        sources: [...new Set(sorted.flatMap((member) => member.sources))].sort(compareUtf8),
        instanceAccount: first.instanceAccount,
        walletAccount: first.walletAccount,
        ...(first.amountArg !== undefined ? { amountArg: first.amountArg } : {}),
      };
    })
    .sort((a, b) => compareUtf8(a.step, b.step));
  merged.forEach((group, i) => {
    if (i > 0 && merged[i - 1]!.step === group.step) {
      throw new StackGenerationError(`stack.activity: the merged step name '${group.step}' is also a step`);
    }
  });
  return merged;
}

const relative = (sources: string[], suffix: string): string[] => sources.map((source) => `${source}.${suffix}`);
const allSources = (groups: StepActivity[]): string[] => groups.flatMap((group) => group.sources).sort(compareUtf8);

/** A JSON string literal for the expression language. */
const literal = (text: string): string => JSON.stringify(text);

// ---------------------------------------------------------------------------
// Programs
// ---------------------------------------------------------------------------

function programDeclaration(spec: ProtocolSpecV1, alias: string, source: StackProgramSource | undefined): Json {
  if (source === undefined) return { source: { registry: kebab(alias) } };
  if ("idl" in source) {
    const program = spec.programs[alias];
    return {
      source: { idl: source.idl },
      ...(program !== undefined ? { program_id: programIdOf(program) } : {}),
    };
  }
  return { source: { registry: source.registry }, version: source.version };
}

function memoDeclaration(source: StackProgramSource): Json {
  if ("idl" in source) return { source: { idl: source.idl }, program_id: MEMO_PROGRAM_ID };
  return { source: { registry: source.registry }, version: source.version };
}

// ---------------------------------------------------------------------------
// Interim profile
// ---------------------------------------------------------------------------

function interimActivityEntity(groups: StepActivity[]): Json {
  const all = allSources(groups);
  const step: Record<string, Json> = {};
  for (const group of groups) {
    step[group.step] = { type: "bool", from: group.sources, expr: "true", strategy: "set_once" };
  }
  const fields: Record<string, Json> = {
    id: {
      fields: {
        occurrence: {
          type: "string",
          from: all,
          expr: 'concat(__signature, ":", __ix_path)',
          strategy: "set_once",
        },
        signature: { from: relative(all, "__signature"), strategy: "set_once" },
        ix_path: { from: relative(all, "__ix_path"), strategy: "set_once" },
      },
    },
    step: { fields: step },
    instance: {
      from: groups.flatMap((g) => relative(g.sources, `accounts.${g.instanceAccount}`)).sort(compareUtf8),
      strategy: "set_once",
    },
    wallet: {
      from: groups.flatMap((g) => relative(g.sources, `accounts.${g.walletAccount}`)).sort(compareUtf8),
      strategy: "set_once",
    },
    slot: { from: relative(all, "__slot"), strategy: "set_once" },
    timestamp: { from: relative(all, "__timestamp"), strategy: "set_once" },
  };
  const amounts = groups
    .filter((g) => g.amountArg !== undefined)
    .flatMap((g) => relative(g.sources, `args.${g.amountArg}`))
    .sort(compareUtf8);
  if (amounts.length > 0) fields.amount = { from: amounts, strategy: "set_once" };
  return {
    primary_key: "id.occurrence",
    indexes: ["instance", "wallet"],
    fields,
    views: {
      byInstance: {
        pipeline: [{ sort: { by: "slot", order: "desc" } }],
        output: "collection",
      },
    },
  };
}

function instanceState(spec: ProtocolSpecV1, primary: string): Record<string, Json> {
  const account = spec.stack.instanceAccounts[primary]!;
  const accountRef = `${account.program}.accounts.${account.account}`;
  const state: Record<string, Json> = {};
  for (const name of sortedKeys(spec.stack.instanceFields)) {
    const field = spec.stack.instanceFields[name]!;
    if (field.account !== primary) continue;
    state[name] = `${accountRef}.${field.field}`;
  }
  return state;
}

function interimInstanceEntity(spec: ProtocolSpecV1, primary: string, groups: StepActivity[]): Json {
  const account = spec.stack.instanceAccounts[primary]!;
  const accountRef = `${account.program}.accounts.${account.account}`;
  const state = instanceState(spec, primary);
  const activity: Record<string, Json> = {};
  for (const group of groups) {
    const lookup = `accounts.${group.instanceAccount}`;
    activity[`${group.step}_count`] = { aggregate: { from: group.sources, op: "count", lookup_by: lookup } };
    activity[`${group.step}_last_slot`] = {
      aggregate: { from: group.sources, op: "max", field: "__slot", lookup_by: lookup },
    };
    if (group.amountArg !== undefined) {
      activity[`${group.step}_amount`] = {
        aggregate: { from: group.sources, op: "sum", field: `args.${group.amountArg}`, lookup_by: lookup },
      };
    }
  }
  assertNoCollisions(Object.keys(activity), "ProtocolInstance.activity");
  const fields: Record<string, Json> = {
    id: { fields: { address: { from: `${accountRef}.__account_address`, strategy: "set_once" } } },
  };
  if (Object.keys(state).length > 0) fields.state = { fields: state };
  if (Object.keys(activity).length > 0) fields.activity = { fields: activity };
  return {
    primary_key: "id.address",
    fields,
    views: {
      byId: {
        pipeline: [{ sort: { by: "id.address", order: "asc" } }],
        output: "collection",
      },
    },
  };
}

/**
 * The accounts the create step binds from one of its ephemeral keys, the
 * instance's own accounts: the only accounts the full profile matches direct
 * activity on (see **Direct activity** in the module comment). A binding
 * qualifies when its expression references `ephemeral.<name>` for a key the
 * create step declares, outside a string literal.
 */
export function instanceOwnedAccounts(spec: ProtocolSpecV1): Set<string> {
  const create = sortedKeys(spec.steps)
    .map((name) => spec.steps[name]!)
    .find((step) => step.scope === "create");
  const owned = new Set<string>();
  if (create?.bind === undefined) return owned;
  const ephemeral = new Set(create.ephemeral);
  for (const [name, expr] of Object.entries(create.bind.accounts)) {
    if (ephemeralReferences(expr).some((key) => ephemeral.has(key))) owned.add(name);
  }
  return owned;
}

const isIdentChar = (code: number): boolean =>
  (code >= 0x30 && code <= 0x39) || (code >= 0x41 && code <= 0x5a) || (code >= 0x61 && code <= 0x7a) || code === 0x5f;

const EPHEMERAL_PREFIX = "ephemeral.";

/** The `<name>` of every `ephemeral.<name>` in an expression, skipping string literals. */
function ephemeralReferences(expr: string): string[] {
  const names: string[] = [];
  let i = 0;
  while (i < expr.length) {
    const code = expr.charCodeAt(i);
    if (code === 0x22 || code === 0x27) {
      // A string literal: skip to its closing quote, honouring escapes.
      i += 1;
      while (i < expr.length && expr.charCodeAt(i) !== code) i += expr.charCodeAt(i) === 0x5c ? 2 : 1;
      i += 1;
      continue;
    }
    const before = i === 0 ? -1 : expr.charCodeAt(i - 1);
    if (expr.startsWith(EPHEMERAL_PREFIX, i) && !isIdentChar(before) && before !== 0x2e) {
      let end = i + EPHEMERAL_PREFIX.length;
      while (end < expr.length && isIdentChar(expr.charCodeAt(end))) end += 1;
      names.push(expr.slice(i + EPHEMERAL_PREFIX.length, end));
      i = end;
      continue;
    }
    i += 1;
  }
  return names;
}

/** The one instruction account each role is read from, across every activity group. */
function sharedAccount(groups: StepActivity[], role: "instanceAccount" | "walletAccount", where: string): string {
  const names = [...new Set(groups.map((group) => group[role]))].sort(compareUtf8);
  if (names.length !== 1) {
    throw new StackGenerationError(
      `${where} needs every stack.activity entry to name the same ${role}; found ${names.join(", ")}`,
    );
  }
  return names[0]!;
}

function interimParticipantEntity(groups: StepActivity[]): Json {
  const all = allSources(groups);
  // A row key is one field (generated SDKs support no composite keys), and
  // its expression reads the same account names from every instruction.
  const instanceAccount = sharedAccount(groups, "instanceAccount", "ProtocolParticipant");
  const walletAccount = sharedAccount(groups, "walletAccount", "ProtocolParticipant");
  const activity: Record<string, Json> = {};
  for (const group of groups) {
    activity[`${group.step}_count`] = { type: "u64", from: group.sources, expr: "1", strategy: "sum" };
    if (group.amountArg !== undefined) {
      activity[`${group.step}_amount`] = { from: relative(group.sources, `args.${group.amountArg}`), strategy: "sum" };
    }
  }
  activity.last_slot = { from: relative(all, "__slot"), strategy: "max" };
  assertNoCollisions(Object.keys(activity), "ProtocolParticipant.activity");
  return {
    primary_key: "id.key",
    indexes: ["id.instance"],
    fields: {
      id: {
        fields: {
          key: {
            type: "string",
            from: all,
            expr: `concat(accounts.${instanceAccount}, ":", accounts.${walletAccount})`,
            strategy: "set_once",
          },
          instance: {
            from: groups.flatMap((g) => relative(g.sources, `accounts.${g.instanceAccount}`)).sort(compareUtf8),
            strategy: "set_once",
          },
          wallet: {
            from: groups.flatMap((g) => relative(g.sources, `accounts.${g.walletAccount}`)).sort(compareUtf8),
            strategy: "set_once",
          },
        },
      },
      activity: { fields: activity },
    },
    views: {
      byInstance: {
        pipeline: [{ sort: { by: "id.instance", order: "asc" } }],
        output: "collection",
      },
    },
  };
}

// ---------------------------------------------------------------------------
// Full profile
// ---------------------------------------------------------------------------

/** Memo v2 field positions, counting from 1 as `split_part` does. */
const MEMO_FIELD = { scope: 4, instance: 5, participant: 6, step: 7 } as const;

interface MemoContext {
  /** `<alias>.instructions.memo`. */
  memo: string;
  /** Holds for an attested Memo v2 of this deployment. */
  attested: string;
  scoped: boolean;
}

const memoPart = (field: keyof typeof MEMO_FIELD): string => `split_part(args.memo, ":", ${MEMO_FIELD[field]})`;

/** A string field written once from an attested memo. */
function memoString(ctx: MemoContext, expr: string, guard = ctx.attested): Json {
  return { type: "string", from: ctx.memo, expr, strategy: "set_once", guard };
}

/** The instance or participant field holding the signature of the last attested memo naming `step`. */
const lastStepSignature = (step: string): string => `last_${step}_signature`;

function fullInstanceEntity(
  spec: ProtocolSpecV1,
  primary: string | undefined,
  groups: ActivityGroup[],
  ctx: MemoContext,
): Json {
  const instanceAccounts = [...new Set(groups.map((group) => group.instanceAccount))];
  if (instanceAccounts.length > 1) {
    throw new StackGenerationError(
      `the full profile routes all activity through one instance account; stack.activity names ${instanceAccounts.join(", ")}`,
    );
  }
  // Direct activity is matched only on the instance's own accounts.
  const direct = instanceAccounts.length === 1 && instanceOwnedAccounts(spec).has(instanceAccounts[0]!);
  const lookup = direct ? `accounts.${instanceAccounts[0]}` : undefined;
  const all = allSources(groups);

  const registrations: Json[] = (direct ? groups : []).flatMap((group) =>
    group.sources.map(
      (source): Json => ({
        instruction: ctx.memo,
        sibling: source,
        pda_field: `sibling.accounts.${group.instanceAccount}`,
        seed_expr: memoPart("instance"),
        guard: ctx.attested,
        lookup_name: "instance_accounts",
      }),
    ),
  );

  const id: Record<string, Json> = { instance: memoString(ctx, memoPart("instance")) };
  if (ctx.scoped) id.scope = memoString(ctx, memoPart("scope"));

  const activity: Record<string, Json> = {};
  // Attested amounts, per step: an instruction adds to a step's amount only
  // in a transaction whose attested memo names that step (joined through
  // `last_<step>_signature`), so it counts once, under the step the memo names.
  const stepSignatures: Record<string, Json> = {};
  const protocolAmount = (step: StepActivity): void => {
    if (step.amountArg === undefined) return;
    stepSignatures[lastStepSignature(step.step)] = {
      type: "string",
      from: ctx.memo,
      expr: "__signature",
      strategy: "last_write",
      guard: `${ctx.attested} && ${memoPart("step")} == ${literal(step.step)}`,
      emit: false,
    };
    activity[`${step.step}_protocol_amount`] = {
      from: relative(step.sources, `args.${step.amountArg}`),
      strategy: "sum",
      lookup_by: "__signature",
      via: lastStepSignature(step.step),
    };
  };
  if (lookup !== undefined) {
    const via = { lookup_by: lookup, via: "instance_accounts" };
    for (const group of groups) {
      // The instructions count once, under the group; the memo names the
      // step, so the attested part is counted per step.
      const total = `(activity.${group.step}_count ?? 0)`;
      const parts = group.steps.map((step) => `(activity.${step.step}_protocol_count ?? 0)`);
      const attested = parts.length === 1 ? parts[0]! : `(${parts.join(" + ")})`;
      activity[`${group.step}_count`] = { aggregate: { from: group.sources, op: "count", ...via } };
      for (const step of group.steps) {
        activity[`${step.step}_protocol_count`] = {
          type: "u64",
          from: ctx.memo,
          expr: "1",
          strategy: "sum",
          guard: `${ctx.attested} && ${memoPart("step")} == ${literal(step.step)}`,
        };
        protocolAmount(step);
      }
      activity[`${group.step}_direct_count`] = {
        type: "u64",
        compute: `if(${total} >= ${attested}, ${total} - ${attested}, 0)`,
      };
      if (group.amountArg !== undefined) {
        activity[`${group.step}_amount`] = {
          aggregate: { from: group.sources, op: "sum", field: `args.${group.amountArg}`, ...via },
        };
      }
    }
    activity.last_signature = { from: relative(all, "__signature"), ...via };
    activity.last_ix_path = { from: relative(all, "__ix_path"), ...via };
    activity.last_slot = { from: relative(all, "__slot"), strategy: "max", ...via };
    activity.last_timestamp = { from: relative(all, "__timestamp"), strategy: "max", ...via };
  } else if (groups.length > 0) {
    // No account belongs to the instance alone, so only attested memos reach
    // the row: per-step protocol counts, and the last protocol transaction.
    for (const step of groups.flatMap((group) => group.steps)) {
      activity[`${step.step}_protocol_count`] = {
        type: "u64",
        from: ctx.memo,
        expr: "1",
        strategy: "sum",
        guard: `${ctx.attested} && ${memoPart("step")} == ${literal(step.step)}`,
      };
      protocolAmount(step);
    }
    activity.last_signature = { from: `${ctx.memo}.__signature`, guard: ctx.attested };
    activity.last_ix_path = { from: `${ctx.memo}.__ix_path`, guard: ctx.attested };
    activity.last_slot = { from: `${ctx.memo}.__slot`, strategy: "max", guard: ctx.attested };
    activity.last_timestamp = { from: `${ctx.memo}.__timestamp`, strategy: "max", guard: ctx.attested };
  }
  assertNoCollisions(Object.keys(activity), "ProtocolInstance.activity");

  const fields: Record<string, Json> = { id: { fields: id }, ...stepSignatures };
  const state = primary !== undefined && lookup !== undefined ? instanceState(spec, primary) : {};
  if (Object.keys(state).length > 0) fields.state = { fields: state };
  if (Object.keys(activity).length > 0) fields.activity = { fields: activity };
  return {
    primary_key: "id.instance",
    ...(ctx.scoped ? { scope: "id.scope" } : {}),
    indexes: ["id.instance", ...Object.keys(stepSignatures).sort(compareUtf8)],
    ...(registrations.length > 0 ? { register_from: registrations } : {}),
    fields,
    views: {
      byId: {
        pipeline: [{ sort: { by: "id.instance", order: "asc" } }],
        output: "collection",
      },
    },
  };
}

/** Fields an activity instruction adds to the row of the attested memo in its transaction. */
function joinedBySignature(groups: StepActivity[], via: string): Record<string, Json> {
  const join = { lookup_by: "__signature", via };
  const fields: Record<string, Json> = {
    wallet: {
      from: groups.flatMap((g) => relative(g.sources, `accounts.${g.walletAccount}`)).sort(compareUtf8),
      strategy: "set_once",
      ...join,
    },
  };
  const amounts = groups
    .filter((g) => g.amountArg !== undefined)
    .flatMap((g) => relative(g.sources, `args.${g.amountArg}`))
    .sort(compareUtf8);
  if (amounts.length > 0) fields.amount = { from: amounts, strategy: "sum", ...join };
  return fields;
}

function fullActivityEntity(groups: ActivityGroup[], ctx: MemoContext): Json {
  const fields: Record<string, Json> = {
    id: { fields: { signature: memoString(ctx, "__signature") } },
    instance: memoString(ctx, memoPart("instance")),
    participant: memoString(ctx, memoPart("participant")),
    step: memoString(ctx, memoPart("step")),
    attested: { type: "bool", from: ctx.memo, expr: "true", strategy: "set_once", guard: ctx.attested },
    via: memoString(ctx, literal("protocol")),
    slot: { from: `${ctx.memo}.__slot`, strategy: "set_once", guard: ctx.attested },
    timestamp: { from: `${ctx.memo}.__timestamp`, strategy: "set_once", guard: ctx.attested },
  };
  if (ctx.scoped) fields.scope = memoString(ctx, memoPart("scope"));
  if (groups.length > 0) {
    Object.assign(fields, joinedBySignature(groups, "id.signature"));
    // The account the step's activity entry names (`instanceAccount`).
    fields.activity_account = {
      from: groups.flatMap((g) => relative(g.sources, `accounts.${g.instanceAccount}`)).sort(compareUtf8),
      strategy: "set_once",
      lookup_by: "__signature",
      via: "id.signature",
    };
  }
  return {
    primary_key: "id.signature",
    ...(ctx.scoped ? { scope: "scope" } : {}),
    indexes: ["id.signature", "instance"],
    fields,
    views: {
      byInstance: {
        pipeline: [{ sort: { by: "slot", order: "desc" } }],
        output: "collection",
      },
    },
  };
}

function fullParticipantEntity(steps: StepActivity[], groups: ActivityGroup[], ctx: MemoContext): Json {
  const withParticipant = `${ctx.attested} && ${memoPart("participant")} != "-"`;
  const activity: Record<string, Json> = {};
  const stepSignatures: Record<string, Json> = {};
  for (const step of steps) {
    const named = `${withParticipant} && ${memoPart("step")} == ${literal(step.step)}`;
    activity[`${step.step}_count`] = { type: "u64", from: ctx.memo, expr: "1", strategy: "sum", guard: named };
    if (step.amountArg !== undefined) {
      // An instruction adds to a step's amount only in a transaction whose
      // attested memo names that step, so an instruction several steps list
      // counts once, under the step the memo names.
      stepSignatures[lastStepSignature(step.step)] = {
        type: "string",
        from: ctx.memo,
        expr: "__signature",
        strategy: "last_write",
        guard: named,
        emit: false,
      };
      activity[`${step.step}_amount`] = {
        from: relative(step.sources, `args.${step.amountArg}`),
        strategy: "sum",
        lookup_by: "__signature",
        via: lastStepSignature(step.step),
      };
    }
  }
  activity.last_slot = { from: `${ctx.memo}.__slot`, strategy: "max", guard: withParticipant };
  assertNoCollisions(Object.keys(activity), "ProtocolParticipant.activity");
  const fields: Record<string, Json> = {
    id: { fields: { participant: memoString(ctx, memoPart("participant"), withParticipant) } },
    instance: memoString(ctx, memoPart("instance"), withParticipant),
    last_signature: {
      type: "string",
      from: ctx.memo,
      expr: "__signature",
      strategy: "last_write",
      guard: withParticipant,
      emit: false,
    },
    ...stepSignatures,
    activity: { fields: activity },
  };
  if (ctx.scoped) fields.scope = memoString(ctx, memoPart("scope"), withParticipant);
  if (groups.length > 0) {
    fields.wallet = {
      from: groups.flatMap((g) => relative(g.sources, `accounts.${g.walletAccount}`)).sort(compareUtf8),
      strategy: "set_once",
      lookup_by: "__signature",
      via: "last_signature",
    };
  }
  return {
    primary_key: "id.participant",
    ...(ctx.scoped ? { scope: "scope" } : {}),
    indexes: ["id.participant", "instance", "last_signature", ...Object.keys(stepSignatures).sort(compareUtf8)],
    fields,
    views: {
      byInstance: {
        pipeline: [{ sort: { by: "instance", order: "asc" } }],
        output: "collection",
      },
    },
  };
}

const SHORT_ID = /^[A-Za-z0-9_-]{1,64}$/;

function memoContext(spec: ProtocolSpecV1, options: GenerateStackSourceOptions): MemoContext {
  const attestation = options.attestation;
  if (attestation === undefined || !isPubkey(attestation)) {
    throw new StackGenerationError(
      "the full profile needs options.attestation: the deployment revision's attestation public key (base58)",
    );
  }
  // `isPubkey` also bounds the text; decode once more so the literal is exactly a key.
  if (decodeBase58(attestation)?.length !== 32) throw new StackGenerationError("options.attestation is not a public key");
  let prefix = MEMO_V2_PREFIX;
  if (options.deploymentShortId !== undefined) {
    if (!SHORT_ID.test(options.deploymentShortId)) {
      throw new StackGenerationError("options.deploymentShortId must be 1-64 characters of [A-Za-z0-9_-]");
    }
    prefix = `${MEMO_V2_PREFIX}${options.deploymentShortId}:`;
  }
  return {
    memo: `${MEMO_PROGRAM_ALIAS}.instructions.memo`,
    attested: `starts_with(args.memo, ${literal(prefix)}) && accounts.signer == ${literal(attestation)} && accounts.signer.is_signer`,
    scoped: spec.instanceVisibility === "account",
  };
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/** Generate the protocol stack source of a validated spec. */
export function generateStackSource(
  spec: ProtocolSpecV1,
  options: GenerateStackSourceOptions = {},
): ProtocolStackSourceV1 {
  const profile: StackProfile = options.profile ?? "interim";
  if (profile !== "interim" && profile !== "full") {
    throw new StackGenerationError(`unknown stack profile '${String(profile)}'`);
  }
  const accounts = sortedKeys(spec.stack.instanceAccounts);
  const primary = accounts[0];
  const steps = groupActivity(spec.stack.activity);
  const groups = mergeSharedSteps(steps);
  if (primary === undefined && groups.length === 0) {
    throw new StackGenerationError(
      "the protocol stack needs stack.instanceAccounts or stack.activity; declare what the stack should track",
    );
  }

  const aliases = new Set<string>([
    ...spec.stack.activity.map((entry) => entry.program),
    ...(primary !== undefined ? [spec.stack.instanceAccounts[primary]!.program] : []),
  ]);
  const programs: Record<string, Json> = {};
  for (const alias of [...aliases].sort(compareUtf8)) {
    programs[alias] = programDeclaration(spec, alias, options.programs?.[alias]);
  }

  const entities: Record<string, Json> = {};
  if (profile === "interim") {
    if (groups.length > 0) {
      entities.ProtocolActivity = interimActivityEntity(groups);
      entities.ProtocolParticipant = interimParticipantEntity(groups);
    }
    if (primary !== undefined) entities.ProtocolInstance = interimInstanceEntity(spec, primary, groups);
  } else {
    if (aliases.has(MEMO_PROGRAM_ALIAS)) {
      throw new StackGenerationError(`the program alias '${MEMO_PROGRAM_ALIAS}' is reserved for the memo program`);
    }
    const ctx = memoContext(spec, options);
    programs[MEMO_PROGRAM_ALIAS] = memoDeclaration(options.memoProgram ?? DEFAULT_MEMO_PROGRAM);
    entities.ProtocolActivity = fullActivityEntity(groups, ctx);
    entities.ProtocolInstance = fullInstanceEntity(spec, primary, groups, ctx);
    entities.ProtocolParticipant = fullParticipantEntity(steps, groups, ctx);
  }

  const base = `${pascal(spec.name)}Protocol`;
  const name = byteLength(base) <= 64 ? base : pascal(spec.name);
  const sortedPrograms: Record<string, Json> = {};
  for (const alias of Object.keys(programs).sort(compareUtf8)) sortedPrograms[alias] = programs[alias]!;
  const stackSource = {
    schema: STACK_SOURCE_SCHEMA_V1,
    name,
    programs: sortedPrograms,
    entities,
    // The full profile joins by signature through per-step indexes; strict
    // lookups resolve each join through the index it names only.
    ...(profile === "full" ? { lookups: "strict" } : {}),
  };
  return {
    schema: PROTOCOL_STACK_SOURCE_SCHEMA_V1,
    generator: {
      name: options.generator?.name ?? STACK_GENERATOR.name,
      version: options.generator?.version ?? STACK_GENERATOR.version,
    },
    profile,
    stackSource: stackSource as ProtocolStackSourceV1["stackSource"],
  };
}
