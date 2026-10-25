/** Where the SDK is running, and the environment it may read. */

/**
 * True in a browser page: both `window` and `document` exist. Web workers,
 * service workers, edge runtimes, SSR (Node/Bun) and Deno have no `document`
 * and count as server-side.
 */
export function isBrowserEnvironment(): boolean {
  const scope = globalThis as { window?: unknown; document?: unknown };
  return typeof scope.window === 'object'
    && scope.window !== null
    && typeof scope.document === 'object'
    && scope.document !== null;
}

interface DenoEnvironment {
  env?: { get?: (name: string) => string | undefined };
  permissions?: {
    querySync?: (descriptor: { name: 'env'; variable: string }) => { state?: string };
  };
}

/**
 * Read an environment variable in Node, Bun or Deno without prompting for or
 * failing on missing permissions. Browsers have no environment.
 */
export function readEnvironmentVariable(name: string): string | undefined {
  try {
    const deno = (globalThis as { Deno?: DenoEnvironment }).Deno;
    if (deno) {
      // Reading without --allow-env throws or prompts; ask first.
      const state = deno.permissions?.querySync?.({ name: 'env', variable: name })?.state;
      if (state !== 'granted') return undefined;
      const value = deno.env?.get?.(name);
      return typeof value === 'string' ? value : undefined;
    }
    const value = (globalThis as { process?: { env?: Record<string, string | undefined> } })
      .process?.env?.[name];
    return typeof value === 'string' ? value : undefined;
  } catch {
    return undefined;
  }
}
