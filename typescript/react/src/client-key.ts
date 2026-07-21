import type { ProgramSdkDefinition, StackDefinition } from './types';
import type { ClientLookupOptions } from './types';

type ProgramMap = Record<string, ProgramSdkDefinition>;
type CacheableProgramDefinition = ProgramSdkDefinition & { readonly definitionHash?: string };

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
 * Generated programs share by their behavior fingerprint. Manual definitions
 * fall back to object identity so definitions with unknown behavior never
 * accidentally share a client.
 */
function getProgramsKey(programs: ProgramMap): string {
  const entries = Object.entries(programs)
    .map(([key, definition]) => [
      key,
      (definition as CacheableProgramDefinition).definitionHash
        ?? getObjectKey(definition as object, 'program'),
    ])
    .sort(([left], [right]) => left.localeCompare(right));
  return entries.length > 0 ? JSON.stringify(entries) : 'programs-empty';
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
