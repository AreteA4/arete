import type { ProgramSdkDefinition, StackDefinition } from './types';
import type { ClientLookupOptions } from './types';

type ProgramMap = Record<string, ProgramSdkDefinition>;

const objectKeys = new WeakMap<object, string>();
let nextObjectKey = 0;

function getObjectKey(value: object, prefix: string): string {
  const existing = objectKeys.get(value);
  if (existing) {
    return existing;
  }

  const key = `${prefix}-${++nextObjectKey}`;
  objectKeys.set(value, key);
  return key;
}

/**
 * Value key for an attached program map: two maps attaching the same programs
 * (by name and program id) select the same client, even if the caller builds a
 * fresh object every render. Falls back to object identity when a definition
 * has neither a name nor a program id.
 */
function getProgramsKey(programs: ProgramMap): string {
  const entries = Object.entries(programs)
    .map(([name, definition]) => `${name}@${definition?.name ?? ''}:${definition?.programId ?? ''}`);
  if (entries.some((entry) => entry.endsWith(':'))) {
    return getObjectKey(programs as object, 'programs');
  }
  return entries.sort().join(',') || 'programs-empty';
}

export function createClientCacheKey<
  TStack extends StackDefinition,
  TPrograms extends ProgramMap | undefined = undefined,
>(
  stack: TStack | undefined,
  options?: ClientLookupOptions<TPrograms>
): string | null {
  if (!stack) {
    return null;
  }

  return [
    getObjectKey(stack as object, 'stack'),
    options?.transport ?? 'ws',
    options?.url ?? stack.endpoints.ws,
    options?.httpUrl ?? stack.endpoints.http ?? '',
    options?.programs
      ? getProgramsKey(options.programs)
      : 'programs-none',
  ].join(':');
}
