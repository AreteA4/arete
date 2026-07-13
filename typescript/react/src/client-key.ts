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
      ? getObjectKey(options.programs as object, 'programs')
      : 'programs-none',
  ].join(':');
}
