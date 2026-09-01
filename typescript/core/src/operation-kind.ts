/**
 * Non-enumerable brand attached by the operation and read factories so
 * tooling (e.g. the knowledge-layer extractor's runtime walk) can classify
 * values by brand instead of duck-typing. Non-enumerable keeps JSON
 * serialization and object spreads of surrounding containers unchanged.
 */
export const OPERATION_KIND = '__areteOperationKind' as const;

export type OperationKindBrand =
  | 'instruction'
  | 'transaction'
  | 'flow'
  | 'account-fetch'
  | 'query';

export function brandOperationKind<T extends object>(value: T, kind: OperationKindBrand): T {
  Object.defineProperty(value, OPERATION_KIND, {
    value: kind,
    enumerable: false,
    writable: false,
    configurable: false,
  });
  return value;
}

export function getOperationKind(value: unknown): string | undefined {
  if (value === null || (typeof value !== 'object' && typeof value !== 'function')) {
    return undefined;
  }
  const kind = (value as { [OPERATION_KIND]?: unknown })[OPERATION_KIND];
  return typeof kind === 'string' ? kind : undefined;
}
