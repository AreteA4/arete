/**
 * Recursively convert bigints to their decimal string form.
 *
 * Streamed entities contain bigints, which makes them unsafe to pass through
 * anything that JSON-stringifies (React's dev-mode performance tooling, log
 * pipelines, `JSON.stringify` persistence). Prefer stack-provided UI fields
 * (e.g. `deployedPerSquareUi`) when they exist; use this for the remaining
 * raw values such as account snapshots.
 *
 * Arrays and plain objects are converted structurally and the return type
 * maps every `bigint` to `string`; class instances (PublicKey, Date, …) are
 * returned untouched.
 */
export type StringifiedBigints<T> = T extends bigint
  ? string
  : T extends readonly (infer U)[]
    ? readonly StringifiedBigints<U>[]
    : T extends object
      ? { [K in keyof T]: StringifiedBigints<T[K]> }
      : T;

function isPlainObject(value: object): boolean {
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

export function stringifyBigints<T>(value: T): StringifiedBigints<T> {
  if (typeof value === 'bigint') {
    return String(value) as StringifiedBigints<T>;
  }
  if (Array.isArray(value)) {
    return value.map((entry) => stringifyBigints(entry)) as StringifiedBigints<T>;
  }
  if (value !== null && typeof value === 'object' && isPlainObject(value)) {
    const out: Record<string, unknown> = {};
    for (const [key, entry] of Object.entries(value)) {
      out[key] = stringifyBigints(entry);
    }
    return out as StringifiedBigints<T>;
  }
  return value as StringifiedBigints<T>;
}
