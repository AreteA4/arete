/** npm package whose `latest` dist-tag tracks the newest `a4` release. */
export const A4_NPM_PACKAGE = '@usearete/a4';

/** Config value that resolves to the newest published release at sweep start. */
export const LATEST = 'latest';

const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;

export type FetchLatest = () => Promise<string>;

/** The `latest` dist-tag of `@usearete/a4` on the npm registry. */
export const fetchLatestA4: FetchLatest = async () => {
  const url = `https://registry.npmjs.org/-/package/${A4_NPM_PACKAGE}/dist-tags`;
  const response = await fetch(url, { signal: AbortSignal.timeout(15_000) });
  if (!response.ok) throw new Error(`HTTP ${response.status} for ${url}`);
  const latest = ((await response.json()) as { latest?: unknown }).latest;
  if (typeof latest !== 'string' || !SEMVER.test(latest)) {
    throw new Error(`${url} returned no usable latest version`);
  }
  return latest;
};

export interface ResolvedA4Version {
  /** Concrete release every run of the sweep installs. */
  version: string;
  /** `latest` when resolved from npm, `pinned` when the config named a version. */
  source: 'latest' | 'pinned';
  /** npm `latest` when it could be read; set for pins too, to spot stale ones. */
  latest?: string;
  /** Why `latest` could not be read, when it could not. */
  lookupError?: string;
}

/** Strip a leading `v`, so `v0.33.0` and `0.33.0` pin the same release. */
export function normalizeVersion(version: string): string {
  return version.trim().replace(/^v(?=\d)/, '');
}

/**
 * Turn the configured `a4Version` into one concrete release. `latest` is
 * looked up once, so every run in a sweep installs the same version; a pin is
 * used as given, and npm `latest` is still read so a stale pin can be
 * reported.
 */
export async function resolveA4Version(
  requested: string,
  fetchLatest: FetchLatest = fetchLatestA4,
): Promise<ResolvedA4Version> {
  const wanted = normalizeVersion(requested);
  let latest: string | undefined;
  let lookupError: string | undefined;
  try {
    latest = await fetchLatest();
  } catch (err) {
    lookupError = err instanceof Error ? err.message : String(err);
  }
  if (wanted === LATEST) {
    if (!latest) {
      throw new Error(`could not resolve the latest ${A4_NPM_PACKAGE} release (${lookupError}); pin one with --a4-version`);
    }
    return { version: latest, source: 'latest', latest };
  }
  if (!SEMVER.test(wanted)) {
    throw new Error(`a4Version must be "latest" or a release such as 0.33.0, got "${requested}"`);
  }
  return { version: wanted, source: 'pinned', ...(latest ? { latest } : {}), ...(lookupError ? { lookupError } : {}) };
}

/** -1, 0 or 1 by numeric major.minor.patch; prerelease tags are ignored. */
export function compareVersions(a: string, b: string): number {
  const parts = (v: string) => v.split('-')[0]!.split('.').map(Number);
  const [pa, pb] = [parts(a), parts(b)];
  for (let i = 0; i < 3; i++) {
    const diff = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (diff) return Math.sign(diff);
  }
  return 0;
}

/** A warning when a pinned version is older than npm `latest`, otherwise undefined. */
export function stalePinWarning(resolved: ResolvedA4Version): string | undefined {
  if (resolved.source !== 'pinned' || !resolved.latest) return undefined;
  if (compareVersions(resolved.version, resolved.latest) >= 0) return undefined;
  return `pinned a4 ${resolved.version} is behind the latest release ${resolved.latest}`;
}
