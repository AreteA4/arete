/**
 * Minimal shape of a named status source: any view hook result, read hook
 * result, or compatible object. Use `false`/`null`/`undefined` to exclude a
 * source conditionally, e.g. `Miner: authority && miner`.
 */
export type ViewStatusSource =
  | { isLoading: boolean; isRefreshing?: boolean; error?: Error | null }
  | false
  | null
  | undefined;

export interface SummarizedViewStatus {
  /** True while any included source is still loading. */
  isLoading: boolean;
  /** True when any included source has an error. */
  hasError: boolean;
  /** True while any included source is refreshing committed data. */
  isRefreshing: boolean;
  /** Names of sources still loading, in declaration order. */
  loading: string[];
  /** `"Name: message"` for each source error, in declaration order. */
  errors: string[];
  /** Names of sources refreshing committed data, in declaration order. */
  refreshing: string[];
}

/**
 * Aggregate the status of connection, view, and read hook results into loading,
 * refreshing, and error lists. Pure function; call it during render.
 *
 * ```tsx
 * const streams = summarizeStatuses({ Board: board, Round: round, Miner: authority && miner });
 * {streams.loading.length > 0 && <p>Loading: {streams.loading.join(', ')}</p>}
 * {streams.errors.length > 0 && <p role="alert">{streams.errors.join(' · ')}</p>}
 * ```
 */
export function summarizeStatuses(
  sources: Record<string, ViewStatusSource>
): SummarizedViewStatus {
  const loading: string[] = [];
  const errors: string[] = [];
  const refreshing: string[] = [];
  for (const [name, source] of Object.entries(sources)) {
    if (!source) continue;
    if (source.isLoading) loading.push(name);
    if (source.isRefreshing) refreshing.push(name);
    if (source.error) errors.push(`${name}: ${source.error.message}`);
  }
  return {
    isLoading: loading.length > 0,
    hasError: errors.length > 0,
    isRefreshing: refreshing.length > 0,
    loading,
    errors,
    refreshing,
  };
}

/** @deprecated Use summarizeStatuses; it also accepts connection and read results. */
export const summarizeViews = summarizeStatuses;
export type StatusSource = ViewStatusSource;
export type SummarizedStatus = SummarizedViewStatus;
