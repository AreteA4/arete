import { useMemo } from 'react';
import {
  createPlatformHttpClient,
  type AuthConfig,
  type PlatformHttpClient,
} from '@usearete/sdk';

import { useAreteContext } from './provider';

export interface UsePlatformHttpClientOptions {
  /** API origin, with or without `/api`. Defaults to `https://api.arete.run`. */
  readonly apiUrl?: string;
  /** Defaults to the provider's `auth`. */
  readonly auth?: AuthConfig;
}

/**
 * A client for the Arete API's account routes, authenticated with the
 * provider's `auth` (and `fetch`). With `auth.tokenKind: 'api'`, the same
 * credential the provider mints its stack and gateway sessions with, so one
 * login drives both:
 *
 * ```tsx
 * <AreteProvider auth={{ tokenKind: 'api', tokenEndpoint: '/api/arete-token' }}>
 *   …
 * </AreteProvider>
 * // in a component
 * const http = usePlatformHttpClient({ apiUrl });
 * const squads = useMemo(() => createSquadsClient({ http }), [http]);
 * ```
 *
 * The client is stable while `apiUrl`, `auth` and `fetch` keep their
 * identity, so pass a memoized (or module-level) `auth` object. Throws
 * during render when the auth carries no API credential, like
 * `createPlatformHttpClient`.
 */
export function usePlatformHttpClient(
  options: UsePlatformHttpClientOptions = {}
): PlatformHttpClient {
  const { config } = useAreteContext();
  const auth = options.auth ?? config.auth;
  const fetchImpl = config.fetch;
  const { apiUrl } = options;
  return useMemo(
    () => createPlatformHttpClient({
      ...(apiUrl !== undefined ? { apiUrl } : {}),
      ...(auth ? { auth } : {}),
      ...(fetchImpl ? { fetch: fetchImpl } : {}),
    }),
    [apiUrl, auth, fetchImpl]
  );
}
