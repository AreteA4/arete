import { AreteError, type ProgramSdkDefinition } from './types';

/** Error code for two different program SDKs claiming one program key. */
export const PROGRAM_KEY_CONFLICT = 'PROGRAM_KEY_CONFLICT' as const;

/**
 * How two program definitions under one key relate:
 *
 * - `'same'`: the same program SDK. They are the same object, or both carry a
 *   `packageReleaseHash` and the hashes are equal.
 * - `'unproven'`: at least one has no `packageReleaseHash` (a local build, or
 *   a program extended outside its generated SDK), but both carry the same
 *   `programSpecHash`. They describe the same program but cannot be proven to
 *   be the same SDK.
 * - `'different'`: both carry a `packageReleaseHash` and the hashes differ, or
 *   neither identity matches (different or missing `programSpecHash`).
 */
export type ProgramIdentityMatch = 'same' | 'unproven' | 'different';

function nonEmpty(value: unknown): string | undefined {
  return typeof value === 'string' && value.length > 0 ? value : undefined;
}

/** Compare two program definitions by identity, never by name. */
export function compareProgramIdentity(
  left: ProgramSdkDefinition,
  right: ProgramSdkDefinition,
): ProgramIdentityMatch {
  if (left === right) return 'same';
  const leftRelease = nonEmpty(left.packageReleaseHash);
  const rightRelease = nonEmpty(right.packageReleaseHash);
  if (leftRelease !== undefined && rightRelease !== undefined) {
    return leftRelease === rightRelease ? 'same' : 'different';
  }
  const leftSpec = nonEmpty(left.programSpecHash);
  return leftSpec !== undefined && leftSpec === nonEmpty(right.programSpecHash)
    ? 'unproven'
    : 'different';
}

/**
 * Whether two program definitions are provably the same program SDK: the same
 * object, or both carry the same `packageReleaseHash` (the program package
 * release they were generated from). Anything else is not provably the same,
 * even under the same name.
 */
export function isSameProgramSdk(
  left: ProgramSdkDefinition,
  right: ProgramSdkDefinition,
): boolean {
  return compareProgramIdentity(left, right) === 'same';
}

function identityOf(program: ProgramSdkDefinition): string {
  const release = nonEmpty(program.packageReleaseHash);
  if (release !== undefined) return `package release ${release}`;
  const spec = nonEmpty(program.programSpecHash);
  return spec !== undefined
    ? `program spec ${spec}, no package release`
    : 'no program identity';
}

const warned = new Set<string>();

/** Emit a program identity warning once per distinct message. */
function warnOnce(message: string): void {
  if (warned.has(message)) return;
  warned.add(message);
  console.warn(`[Arete] ${message}`);
}

/**
 * A program attached next to a stack (`withPrograms`, `ConnectOptions.programs`,
 * `useArete(stack, { programs })`, a session member's `programs`) under a key
 * the stack already provides with a different program SDK.
 */
export function attachedProgramKeyConflict(
  stackName: string,
  key: string,
  stackProgram: ProgramSdkDefinition,
  attached: ProgramSdkDefinition,
): AreteError {
  return new AreteError(
    `Program key '${key}' conflicts with stack '${stackName}': the stack already provides `
      + `a different '${key}' program SDK (stack: ${identityOf(stackProgram)}; attached: `
      + `${identityOf(attached)}). Use the stack's program at programs.${key}, or attach the `
      + `other program under a different key, one the stack does not use.`,
    PROGRAM_KEY_CONFLICT,
    { key, stack: stackName },
  );
}

/**
 * An attached program replaces the stack's program under one key without a
 * provable identity match (same program spec, no shared package release).
 */
export function warnUnprovenAttachedProgram(
  stackName: string,
  key: string,
  stackProgram: ProgramSdkDefinition,
  attached: ProgramSdkDefinition,
): void {
  warnOnce(
    `programs.${key} uses the program attached to stack '${stackName}': it has the same `
      + `program spec as the stack's '${key}' program but could not be proven identical `
      + `(stack: ${identityOf(stackProgram)}; attached: ${identityOf(attached)}).`,
  );
}

/** A standalone session program under a key a session stack already provides. */
export function sessionProgramKeyConflict(
  stackKey: string,
  key: string,
  stackProgram: ProgramSdkDefinition,
  standalone: ProgramSdkDefinition,
): AreteError {
  return new AreteError(
    `Program key '${key}' conflicts with stack '${stackKey}' in this session: the stack `
      + `already provides a different '${key}' program SDK (stack: ${identityOf(stackProgram)}; `
      + `standalone: ${identityOf(standalone)}). Use session.stacks.${stackKey}.programs.${key}, `
      + `or attach the standalone program under a different key in createSession({ programs }).`,
    PROGRAM_KEY_CONFLICT,
    { key, stack: stackKey },
  );
}

/**
 * A standalone session program takes the top-level key from stacks whose
 * program it could not be proven identical to.
 */
export function warnUnprovenSessionProgram(
  key: string,
  standalone: ProgramSdkDefinition,
  providers: readonly { readonly stackKey: string; readonly program: ProgramSdkDefinition }[],
): void {
  const stacks = providers.map(({ stackKey }) => `'${stackKey}'`).join(' and ');
  const paths = providers
    .map(({ stackKey }) => `session.stacks.${stackKey}.programs.${key}`)
    .join(' and ');
  warnOnce(
    `session.programs.${key} uses the standalone program: it has the same program spec as `
      + `the '${key}' program of stack ${stacks} but could not be proven identical `
      + `(standalone: ${identityOf(standalone)}; ${providers
        .map(({ stackKey, program }) => `${stackKey}: ${identityOf(program)}`)
        .join('; ')}). ${paths} keep${providers.length === 1 ? 's' : ''} the stack's program.`,
  );
}

/**
 * Several session stacks bundle one key with the same program spec, not all
 * provably the same SDK; the first stack's program takes the top-level key.
 */
export function warnUnprovenStackPrograms(
  key: string,
  providers: readonly { readonly stackKey: string; readonly program: ProgramSdkDefinition }[],
): void {
  const first = providers[0]!;
  warnOnce(
    `session.programs.${key} uses stack '${first.stackKey}''s program: stacks `
      + `${providers.map(({ stackKey }) => `'${stackKey}'`).join(' and ')} bundle '${key}' `
      + `programs with the same program spec that could not be proven identical (${providers
        .map(({ stackKey, program }) => `${stackKey}: ${identityOf(program)}`)
        .join('; ')}). Each stays at session.stacks.<stack>.programs.${key}.`,
  );
}

/** Two session stacks provide different program SDKs under one key. */
export function ambiguousSessionProgram(
  key: string,
  providers: readonly { readonly stackKey: string; readonly program: ProgramSdkDefinition }[],
): AreteError {
  const stacks = providers.map(({ stackKey }) => `'${stackKey}'`).join(' and ');
  const paths = providers
    .map(({ stackKey }) => `session.stacks.${stackKey}.programs.${key}`)
    .join(' or ');
  const identities = providers
    .map(({ stackKey, program }) => `${stackKey}: ${identityOf(program)}`)
    .join('; ');
  return new AreteError(
    `session.programs.${key} is ambiguous: stacks ${stacks} provide different '${key}' `
      + `program SDKs (${identities}). Use ${paths}, or attach the one you want under a `
      + `different key in createSession({ programs }).`,
    PROGRAM_KEY_CONFLICT,
    { key, stacks: providers.map(({ stackKey }) => stackKey) },
  );
}
