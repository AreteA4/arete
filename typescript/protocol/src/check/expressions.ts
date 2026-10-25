/**
 * Local expression checks for authors: syntax and names only.
 *
 * Guards, derivations, reads, templates, bindings and reaction conditions
 * use the protocol expression language. This module tokenizes that syntax,
 * extracts identifier paths and function names, and checks each reference
 * against what is in scope at its use site, so an author learns about
 * `reads.escorw` or `ephemeral.mint` on a step that allocates no `mint`
 * before uploading. It does not parse precedence or check types: the
 * server's check (`arete-protocol check` with a secret key, or
 * `a4 protocol deploy --dry-run`) is the reference, and this module must
 * never reject what the server accepts.
 */

import { sortedKeys } from "../contracts/shape.js";
import type { MachineKind, ProtocolSpecV1, StepV1 } from "../contracts/types.js";

export type ExpressionFindingCode =
  | "unknown-reference"
  | "unknown-function"
  | "invalid-expression"
  | "squads-multisig-not-from-resources";

export interface ExpressionFinding {
  code: ExpressionFindingCode;
  /** JSON Pointer to the expression in the spec. */
  path: string;
  /** UTF-16 offset and length of the offending token within the expression. */
  offset: number;
  length: number;
  message: string;
}

/** What an expression may reference at one use site. */
export interface ExpressionScope {
  now: boolean;
  wallet: boolean;
  params: ReadonlySet<string>;
  /** `undefined` where args are not in scope (machine and reaction sites). */
  args?: ReadonlySet<string>;
  /** `undefined` where no instance exists yet (create steps). */
  instance?: { accounts: ReadonlySet<string>; roles: ReadonlySet<string> };
  participant: boolean;
  ephemeral: ReadonlySet<string>;
  reads: ReadonlySet<string>;
  resources: ReadonlySet<string>;
}

/**
 * Every function the server's checker accepts in protocols, as its exported
 * expression reference lists them (a test compares the two).
 */
export const EXPRESSION_FUNCTIONS = [
  "if",
  "some",
  "exists",
  "is_some",
  "is_none",
  "is_null",
  "abs",
  "min",
  "max",
  "ui_amount",
  "raw_amount",
  "concat",
  "starts_with",
  "split_part",
  "utf8",
  "len",
  "sum",
  "index",
  "slice",
  "u64_from_le_bytes",
  "u64_from_be_bytes",
  "keccak256",
  "slot",
  "timestamp",
  "pda",
  "ata",
  "u8",
  "u16",
  "u32",
  "u64",
  "u128",
  "i8",
  "i16",
  "i32",
  "i64",
  "i128",
  "f32",
  "f64",
  "string",
  "bool",
] as const;

/**
 * Fields of the `deployment` root, valid in every expression (including
 * create steps): the deployment id, its attestation pubkey and the Squads
 * proposer pubkey for the calling integrator account (`null` when no step
 * uses `squadsProposer`).
 */
export const DEPLOYMENT_FIELDS: ReadonlySet<string> = new Set(["id", "attestation", "squadsProposer"]);

const LITERAL_WORDS = new Set(["true", "false", "null", "none"]);
const OPERATORS = ["??", "||", "&&", "==", "!=", "<=", ">=", "<<", ">>", "|", "^", "&", "<", ">", "+", "-", "*", "/", "%", "!", "(", ")", ",", "[", "]"];

export type ExpressionToken =
  | { kind: "identifier"; text: string; offset: number }
  | { kind: "string"; text: string; offset: number }
  | { kind: "number"; text: string; offset: number }
  | { kind: "operator"; text: string; offset: number };

/** Tokenize an expression; returns `{ error }` for a character the syntax does not have. */
export function tokenizeExpression(source: string): ExpressionToken[] | { error: { offset: number; length: number; message: string } } {
  const tokens: ExpressionToken[] = [];
  let i = 0;
  while (i < source.length) {
    const c = source[i]!;
    if (/\s/.test(c)) {
      i += 1;
      continue;
    }
    if (c === '"') {
      let k = i + 1;
      while (k < source.length && source[k] !== '"') k += source[k] === "\\" ? 2 : 1;
      if (k >= source.length) return { error: { offset: i, length: source.length - i, message: "unterminated string literal" } };
      tokens.push({ kind: "string", text: source.slice(i, k + 1), offset: i });
      i = k + 1;
      continue;
    }
    const number = /^[0-9]+(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?/.exec(source.slice(i));
    if (number) {
      tokens.push({ kind: "number", text: number[0], offset: i });
      i += number[0].length;
      continue;
    }
    const ident = /^[A-Za-z_][A-Za-z0-9_.]*/.exec(source.slice(i));
    if (ident) {
      tokens.push({ kind: "identifier", text: ident[0], offset: i });
      i += ident[0].length;
      continue;
    }
    const operator = OPERATORS.find((op) => source.startsWith(op, i));
    if (operator) {
      tokens.push({ kind: "operator", text: operator, offset: i });
      i += operator.length;
      continue;
    }
    return { error: { offset: i, length: 1, message: `unexpected character '${c}'` } };
  }
  return tokens;
}

function referenceProblem(segments: string[], scope: ExpressionScope): string | undefined {
  const [root, second] = segments;
  const field = (set: ReadonlySet<string> | undefined, what: string) => {
    if (set === undefined) return `${root} is not in scope here`;
    if (second === undefined) return `name a ${what}: ${root}.<name>`;
    return set.has(second) ? undefined : `${what} '${second}' is not declared here`;
  };
  switch (root) {
    case "now":
      if (segments.length > 1) return "now has no fields";
      return scope.now ? undefined : "now is not in scope here";
    case "wallet":
      if (segments.length > 1) return "wallet has no fields";
      return scope.wallet ? undefined : "wallet is not in scope here (keeper and admin steps have no wallet)";
    case "params":
      return field(scope.params, "param");
    case "args":
      return field(scope.args, "arg");
    case "ephemeral":
      if (segments.length > 2) return "ephemeral keys have no fields";
      return field(scope.ephemeral, "ephemeral key");
    case "reads":
      return field(scope.reads, "read");
    case "resources":
      return field(scope.resources, "resource");
    case "instance": {
      if (scope.instance === undefined) return "instance is not in scope here (create steps run before it exists)";
      if (second === "state" || second === "id") return segments.length === 2 ? undefined : `instance.${second} has no fields`;
      if (second === "params") return undefined;
      const third = segments[2];
      if (second === "accounts") {
        if (third === undefined) return "name a bound account: instance.accounts.<name>";
        if (segments.length > 3) return "bound accounts have no fields";
        return scope.instance.accounts.has(third) ? undefined : `bound account '${third}' is not bound by the create step`;
      }
      if (second === "roles") {
        if (third === undefined) return "name a role: instance.roles.<role>";
        if (segments.length > 3) return "roles have no fields";
        return scope.instance.roles.has(third) ? undefined : `role '${third}' is not declared`;
      }
      return "instance exposes state, id, accounts.<name>, roles.<role> and params.*";
    }
    case "deployment":
      if (second === undefined) return "name a deployment field: deployment.id, deployment.attestation or deployment.squadsProposer";
      if (segments.length > 2 || !DEPLOYMENT_FIELDS.has(second)) {
        return "deployment exposes id, attestation and squadsProposer";
      }
      return undefined;
    case "participant":
      if (!scope.participant) return "participant is not in scope here";
      if (segments.length === 2 && (second === "state" || second === "wallet" || second === "id")) return undefined;
      return "participant exposes state, wallet and id";
    default:
      return `unknown name '${root}'`;
  }
}

/** Check one expression against a scope. `path` labels the findings. */
export function checkExpression(source: string, scope: ExpressionScope, path = ""): ExpressionFinding[] {
  const tokens = tokenizeExpression(source);
  if (!Array.isArray(tokens)) return [{ code: "invalid-expression", path, ...tokens.error }];
  const findings: ExpressionFinding[] = [];
  tokens.forEach((token, i) => {
    if (token.kind !== "identifier") return;
    const next = tokens[i + 1];
    if (next?.kind === "operator" && next.text === "(") {
      if (!(EXPRESSION_FUNCTIONS as readonly string[]).includes(token.text)) {
        findings.push({
          code: "unknown-function",
          path,
          offset: token.offset,
          length: token.text.length,
          message: `unknown function '${token.text}'`,
        });
      }
      return;
    }
    if (LITERAL_WORDS.has(token.text)) return;
    const segments = token.text.split(".");
    if (segments.some((segment) => segment.length === 0)) {
      findings.push({
        code: "invalid-expression",
        path,
        offset: token.offset,
        length: token.text.length,
        message: `'${token.text}' has an empty path segment`,
      });
      return;
    }
    const problem = referenceProblem(segments, scope);
    if (problem !== undefined) {
      findings.push({ code: "unknown-reference", path, offset: token.offset, length: token.text.length, message: problem });
    }
  });
  return findings;
}

const set = (values: Iterable<string>) => new Set(values);

/** The scopes every expression site of a spec sees, and their findings. */
export function checkSpecExpressions(spec: ProtocolSpecV1): ExpressionFinding[] {
  const create = sortedKeys(spec.steps)
    .map((name) => spec.steps[name]!)
    .find((step) => step.scope === "create");
  const bound = set(Object.keys(create?.bind?.accounts ?? {}));
  const instance = { accounts: bound, roles: set(Object.keys(spec.roles)) };
  const params = set(Object.keys(spec.params.properties));
  const resources = set(Object.keys(spec.resources));
  const none = set([]);
  const findings: ExpressionFinding[] = [];
  const check = (source: string, scope: ExpressionScope, path: string) =>
    findings.push(...checkExpression(source, scope, path));

  const machineScope = (kind: MachineKind): ExpressionScope => ({
    now: true,
    // A participant machine is evaluated for one wallet.
    wallet: kind === "participant",
    params,
    instance,
    participant: kind === "participant",
    ephemeral: none,
    reads: set(spec.machines[kind]?.reads ?? []),
    resources,
  });
  for (const kind of ["instance", "participant"] as const) {
    const machine = spec.machines[kind];
    if (machine === undefined) continue;
    machine.derive.forEach((rule, i) => {
      check(rule.when, machineScope(kind), `/machines/${kind}/derive/${i}/when`);
      if (rule.at !== undefined) check(rule.at, machineScope(kind), `/machines/${kind}/derive/${i}/at`);
    });
  }

  // The server checks a read's address once for every step or machine that
  // uses the read, with that user's names minus `now` and `reads`; a read
  // nobody uses is not checked.
  for (const name of sortedKeys(spec.reads)) {
    const users: ExpressionScope[] = [];
    for (const stepName of sortedKeys(spec.steps)) {
      const step = spec.steps[stepName]!;
      if (step.reads.includes(name)) users.push(stepScope(spec, step, { params, instance, resources }));
    }
    for (const kind of ["instance", "participant"] as const) {
      if (spec.machines[kind]?.reads.includes(name)) users.push(machineScope(kind));
    }
    const seen = new Set<string>();
    for (const user of users) {
      for (const finding of checkExpression(spec.reads[name]!.address, { ...user, now: false, reads: none }, `/reads/${name}/address`)) {
        const key = `${finding.offset}:${finding.message}`;
        if (seen.has(key)) continue;
        seen.add(key);
        findings.push({ ...finding, message: `${finding.message} (where the read is used)` });
      }
    }
  }

  spec.reactions.forEach((reaction, i) => {
    const trigger = reaction.trigger;
    if (trigger.kind === "at") check(trigger.time, machineScope(trigger.machine), `/reactions/${i}/trigger/time`);
    if (trigger.kind === "when") {
      check(trigger.condition, machineScope(trigger.machine), `/reactions/${i}/trigger/condition`);
    }
  });

  for (const name of sortedKeys(spec.steps)) {
    const step = spec.steps[name]!;
    const base = `/steps/${name}`;
    const scope = stepScope(spec, step, { params, instance, resources });
    if (step.authority.kind === "squadsProposer") {
      findings.push(...checkSquadsMultisig(step.authority.multisig, resources, `${base}/authority/multisig`));
    }
    step.guards.forEach((guard, i) => check(guard.expr, scope, `${base}/guards/${i}/expr`));
    if (step.fee.kind === "input" && step.fee.asset.kind === "spl") check(step.fee.asset.mint, scope, `${base}/fee/asset/mint`);
    for (const account of sortedKeys(step.bind?.accounts ?? {})) {
      check(step.bind!.accounts[account]!, scope, `${base}/bind/accounts/${account}`);
    }
    for (const role of sortedKeys(step.bind?.roles ?? {})) {
      check(step.bind!.roles[role]!, scope, `${base}/bind/roles/${role}`);
    }
    step.glue.atas.forEach((ata, i) => {
      check(ata.owner, scope, `${base}/glue/atas/${i}/owner`);
      check(ata.mint, scope, `${base}/glue/atas/${i}/mint`);
    });
    if (step.glue.wrapSol !== undefined) check(step.glue.wrapSol.amount, scope, `${base}/glue/wrapSol/amount`);
    if (step.build.kind === "template") {
      step.build.instructions.forEach((instruction, i) => {
        for (const key of ["args", "accounts"] as const) {
          for (const field of sortedKeys(instruction[key])) {
            check(instruction[key][field]!, scope, `${base}/build/instructions/${i}/${key}/${field}`);
          }
        }
        (instruction.remainingAccounts ?? []).forEach((account, k) => {
          check(account.pubkey, scope, `${base}/build/instructions/${i}/remainingAccounts/${k}/pubkey`);
        });
      });
    }
  }
  return findings;
}

/**
 * A Squads step's multisig comes from the deployment config: an expression
 * over `resources.*` and constants only, so it is known (and can be checked)
 * before the deployment activates.
 */
function checkSquadsMultisig(source: string, resources: ReadonlySet<string>, path: string): ExpressionFinding[] {
  const none = set([]);
  const scope: ExpressionScope = { now: false, wallet: false, params: none, participant: false, ephemeral: none, reads: none, resources };
  const tokens = tokenizeExpression(source);
  if (!Array.isArray(tokens)) return checkExpression(source, scope, path);
  const outside: ExpressionFinding[] = [];
  let readsResource = false;
  tokens.forEach((token, i) => {
    if (token.kind !== "identifier" || LITERAL_WORDS.has(token.text)) return;
    const next = tokens[i + 1];
    if (next?.kind === "operator" && next.text === "(") return;
    if (token.text.split(".")[0] === "resources") {
      readsResource = true;
      return;
    }
    outside.push({
      code: "squads-multisig-not-from-resources",
      path,
      offset: token.offset,
      length: token.text.length,
      message: `a Squads step's multisig must come from the deployment config: declare it as a resource and read it with resources.<name>, not '${token.text}'`,
    });
  });
  // Out-of-scope roots get the Squads finding above instead; every problem
  // under `resources` (including a bare `resources`) keeps its own.
  const inScope = checkExpression(source, scope, path).filter(
    (finding) =>
      finding.code !== "unknown-reference" ||
      source.slice(finding.offset, finding.offset + finding.length).split(".")[0] === "resources",
  );
  const findings = [...inScope, ...outside].sort((a, b) => a.offset - b.offset);
  if (findings.length === 0 && !readsResource) {
    findings.push({
      code: "squads-multisig-not-from-resources",
      path,
      offset: 0,
      length: source.length,
      message: "a Squads step's multisig must come from the deployment config: declare it as a resource and read it with resources.<name>",
    });
  }
  return findings;
}

function stepScope(
  _spec: ProtocolSpecV1,
  step: StepV1,
  shared: Pick<ExpressionScope, "params" | "resources"> & { instance: NonNullable<ExpressionScope["instance"]> },
): ExpressionScope {
  return {
    now: true,
    // The server gives every step site `wallet`; a keeper or admin step has
    // none at run time, but the check accepts it, so this check does too.
    wallet: true,
    params: shared.params,
    args: set(Object.keys(step.args.properties)),
    instance: step.scope === "create" ? undefined : shared.instance,
    participant: step.scope === "participant",
    ephemeral: set(step.ephemeral),
    reads: set(step.reads),
    resources: shared.resources,
  };
}

/**
 * Templates that call a builtin program (`system`, `splToken`,
 * `associatedToken`, `computeBudget`, `memo`). Builtins have no program spec
 * yet, so the server cannot check these templates and rejects them
 * (`template_program_spec_missing`). One note per instruction.
 */
export function builtinTemplateWarnings(spec: ProtocolSpecV1): string[] {
  const warnings: string[] = [];
  for (const name of sortedKeys(spec.steps)) {
    const build = spec.steps[name]!.build;
    if (build.kind !== "template") continue;
    build.instructions.forEach((instruction, i) => {
      const program = spec.programs[instruction.program];
      if (program?.kind !== "builtin") return;
      warnings.push(
        `/steps/${name}/build/instructions/${i}: '${instruction.program}' is a builtin program, which has no program spec yet, so the server cannot check this template and rejects it (template_program_spec_missing). Use the step's glue for token accounts, or install the program from the registry and call it under that alias`,
      );
    });
  }
  return warnings;
}
