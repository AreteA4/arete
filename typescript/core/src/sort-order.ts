import type { SortOrder } from './frame';

/** Whether a sort value cannot be ranked: it is missing or `null`. */
export function isUnrankedSortValue(value: unknown): value is null | undefined {
  return value === undefined || value === null;
}

/**
 * Compares two ranked sort values in ascending order. Numbers and bigints
 * compare numerically (also against each other), booleans as `false < true`,
 * and everything else with `localeCompare` on its string form (the SDK
 * ordering rule, shared with the Rust and Python SDKs).
 *
 * This is not the server's ranking. The server compares strings by byte
 * order, except that two decimal-integer strings compare numerically, and it
 * ranks numbers below strings and booleans below both. The server's order
 * decides which entities fill a `take` window; this one only orders the
 * members the client already holds, so the two can list a window's members
 * differently, never choose different members.
 */
export function compareSortValues(left: unknown, right: unknown): number {
  if (left === right) return 0;
  if (isUnrankedSortValue(left)) return isUnrankedSortValue(right) ? 0 : 1;
  if (isUnrankedSortValue(right)) return -1;
  if (typeof left === 'number' && typeof right === 'number') return left - right;
  if (typeof left === 'bigint' && typeof right === 'bigint') return left < right ? -1 : 1;
  if (typeof left === 'bigint' && typeof right === 'number' && Number.isInteger(right)) {
    const other = BigInt(right);
    return left === other ? 0 : left < other ? -1 : 1;
  }
  if (typeof left === 'number' && typeof right === 'bigint' && Number.isInteger(left)) {
    const other = BigInt(left);
    return other === right ? 0 : other < right ? -1 : 1;
  }
  if (typeof left === 'boolean' && typeof right === 'boolean') return Number(left) - Number(right);
  return String(left).localeCompare(String(right));
}

/**
 * Orders two entities' sort values for a view sorted in `order`.
 *
 * A missing or `null` sort value cannot be ranked, so it sorts after every
 * ranked value in both directions: `desc` reverses only the comparison between
 * two ranked values. Two unranked values tie. Callers break ties on the entity
 * key, ascending, whatever the order. The unranked-last rule is the server's
 * and the Rust and Python SDKs'; how two ranked values compare is
 * `compareRanked` (by default {@link compareSortValues}, which differs from
 * the server as described there).
 */
export function compareSortOrder(
  left: unknown,
  right: unknown,
  order: SortOrder,
  compareRanked: (left: unknown, right: unknown) => number = compareSortValues,
): number {
  const leftUnranked = isUnrankedSortValue(left);
  const rightUnranked = isUnrankedSortValue(right);
  if (leftUnranked || rightUnranked) return Number(leftUnranked) - Number(rightUnranked);
  const compared = compareRanked(left, right);
  return order === 'desc' ? -compared : compared;
}
