import { AreteError, type StackDefinition } from './types';

/** Error code for two different stack SDKs claiming one stack key in a session. */
export const STACK_KEY_CONFLICT = 'STACK_KEY_CONFLICT' as const;

function nonEmpty(value: unknown): string | undefined {
  return typeof value === 'string' && value.length > 0 ? value : undefined;
}

/**
 * Whether two stack definitions are provably the same stack SDK: the same
 * object, both carry the same `packageReleaseHash`, or (when either lacks
 * one, as a stack installed on its own does) both were generated for the
 * same served release (`release`).
 */
export function isSameStackSdk(left: StackDefinition, right: StackDefinition): boolean {
  if (left === right) return true;
  const leftPackage = nonEmpty(left.packageReleaseHash);
  const rightPackage = nonEmpty(right.packageReleaseHash);
  if (leftPackage !== undefined && rightPackage !== undefined) {
    return leftPackage === rightPackage;
  }
  const leftManifest = nonEmpty(left.release?.stackManifestHash);
  return leftManifest !== undefined
    && leftManifest === nonEmpty(right.release?.stackManifestHash)
    && left.release?.liveAlias === right.release?.liveAlias;
}

function identityOf(stack: StackDefinition): string {
  const release = nonEmpty(stack.packageReleaseHash);
  if (release !== undefined) return `package release ${release}`;
  const manifest = nonEmpty(stack.release?.stackManifestHash);
  return manifest !== undefined ? `stack manifest ${manifest}` : 'no stack identity';
}

/** Two sources put different stack SDKs under one session stack key. */
export function sessionStackKeyConflict(
  key: string,
  existingSource: string,
  existing: StackDefinition,
  incomingSource: string,
  incoming: StackDefinition,
): AreteError {
  return new AreteError(
    `Stack key '${key}' conflicts in this session: ${existingSource} and ${incomingSource} `
      + `provide different stack SDKs under it (${existingSource}: ${identityOf(existing)}; `
      + `${incomingSource}: ${identityOf(incoming)}). Install the same stack version for both, `
      + `or give the explicit stack a different key in createSession({ stacks }).`,
    STACK_KEY_CONFLICT,
    { key, sources: [existingSource, incomingSource] },
  );
}
