import { createChainClient, type ChainClient } from './chain';
import { ConnectionManager } from './connection';
import {
  createTransactionTransport,
  type TransactionAuthScope,
  type TransactionTransport,
} from './transactions';
import {
  AreteError,
  parseErrorCode,
  shouldRefreshToken,
  type AuthConfig,
  type AuthTokenRequest,
  type HostedSolanaGatewayBindings,
  type HostedSolanaGatewayCapabilityBinding,
  type SolanaGatewayAuthScope,
  type SolanaGatewayBindingAuthTarget,
} from './types';

export interface HostedSolanaGatewayTransportOptions {
  readonly auth?: AuthConfig;
  readonly fetch?: typeof fetch;
}

export interface HostedSolanaGatewayTransports {
  readonly chain: ChainClient;
  readonly transactions: TransactionTransport;
}

interface BindingAuthentication {
  readonly staticToken?: string;
  readonly manager?: ConnectionManager;
  readonly target: SolanaGatewayBindingAuthTarget;
}

function isSecureOrLoopbackHttpUrl(value: string): boolean {
  try {
    const url = new URL(value);
    return url.protocol === 'https:'
      || (url.protocol === 'http:'
        && ['localhost', '127.0.0.1', '::1'].includes(url.hostname));
  } catch {
    return false;
  }
}

function validateBinding(
  capability: 'chain' | 'transactions',
  binding: HostedSolanaGatewayCapabilityBinding,
  requiredScopes: readonly SolanaGatewayAuthScope[]
): void {
  const auth = binding?.auth;
  const complete = binding
    && isSecureOrLoopbackHttpUrl(binding.endpoint)
    && /^sgb_[A-Za-z0-9_-]{32}$/.test(binding.solanaGatewayBindingId)
    && binding.cluster.trim().length > 0
    && binding.region.trim().length > 0
    && auth
    && typeof auth.required === 'boolean'
    && auth.mode === binding.authPolicy
    && isSecureOrLoopbackHttpUrl(auth.sessionEndpoint)
    && isSecureOrLoopbackHttpUrl(auth.jwksUrl)
    && auth.tokenTransport === 'bearer'
    && auth.audience === 'arete:solana-gateway'
    && auth.targetKind === 'solana-gateway-binding'
    && auth.targetId === binding.solanaGatewayBindingId
    && Array.isArray(auth.scopes)
    && requiredScopes.every((scope) => auth.scopes.includes(scope))
    && Array.isArray(auth.acceptedKeyClasses)
    && typeof auth.transactionEntitlementRequired === 'boolean';
  if (!complete) {
    throw new AreteError(
      `Hosted Solana gateway ${capability} binding is incomplete or inconsistent`,
      'INVALID_CONFIG'
    );
  }
}

function resolveFetch(fetchImpl: typeof fetch | undefined): typeof fetch {
  if (fetchImpl) return fetchImpl;
  if (typeof globalThis.fetch === 'function') return globalThis.fetch.bind(globalThis);
  throw new AreteError(
    'A fetch implementation is required for hosted Solana gateway transports',
    'INVALID_CONFIG'
  );
}

function hasRuntimeAuthStrategy(auth: AuthConfig | undefined): boolean {
  return auth?.token !== undefined
    || auth?.getToken !== undefined
    || auth?.tokenEndpoint !== undefined;
}

function bindingAuthConfig(
  binding: HostedSolanaGatewayCapabilityBinding,
  runtimeAuth: AuthConfig | undefined
): AuthConfig | undefined {
  if (hasRuntimeAuthStrategy(runtimeAuth)) return runtimeAuth;
  if (!binding.auth.required) return runtimeAuth;
  return { ...runtimeAuth, tokenEndpoint: binding.auth.sessionEndpoint };
}

function refreshableErrorCode(response: Response): boolean {
  const wireCode = response.headers.get('X-Error-Code');
  return wireCode !== null
    && shouldRefreshToken(parseErrorCode(wireCode.trim().replace(/_/g, '-')));
}

/**
 * Construct explicit hosted chain and transaction transports from generated
 * gateway descriptors. Tokens are isolated by exact binding target and scope.
 */
export function createHostedSolanaGatewayTransports(
  bindings: HostedSolanaGatewayBindings,
  options: HostedSolanaGatewayTransportOptions = {}
): HostedSolanaGatewayTransports {
  validateBinding('chain', bindings.chain, ['read']);
  validateBinding(
    'transactions',
    bindings.transactions,
    ['transaction:inspect', 'transaction:send']
  );

  const fetchImpl = resolveFetch(options.fetch);
  const managers = new Map<string, ConnectionManager>();
  const bindingAuthentication = (
    binding: HostedSolanaGatewayCapabilityBinding
  ): BindingAuthentication => {
    const auth = bindingAuthConfig(binding, options.auth);
    if (auth?.token !== undefined) {
      if (!auth.token) {
        throw new AreteError('Authentication token is empty', 'TOKEN_INVALID');
      }
      return {
        staticToken: auth.token,
        target: {
          targetKind: 'solana-gateway-binding',
          targetId: binding.solanaGatewayBindingId,
        },
      };
    }

    let manager: ConnectionManager | undefined;
    if (auth && (auth.getToken || auth.tokenEndpoint)) {
      const identity = hasRuntimeAuthStrategy(options.auth)
        ? 'runtime-auth-strategy'
        : `session-endpoint:${binding.auth.sessionEndpoint}`;
      manager = managers.get(identity);
      if (!manager) {
        manager = new ConnectionManager({ websocketUrl: null, auth, fetch: fetchImpl });
        managers.set(identity, manager);
      }
    }
    return {
      manager,
      target: {
        targetKind: 'solana-gateway-binding',
        targetId: binding.solanaGatewayBindingId,
      },
    };
  };

  const authenticatedFetch = (authentication: BindingAuthentication) => async (
    input: string,
    init: RequestInit | undefined,
    scope: SolanaGatewayAuthScope,
    requirePreDispatchMarker: boolean
  ): Promise<Response> => {
    const request: AuthTokenRequest = { ...authentication.target, scopes: [scope] };
    const attempt = async (forceRefresh: boolean): Promise<Response> => {
      const token = authentication.staticToken
        ?? (authentication.manager
          ? await authentication.manager.getHttpAuthToken(request, forceRefresh)
          : undefined);
      const headers = new Headers(init?.headers);
      if (token) headers.set('authorization', `Bearer ${token}`);
      return fetchImpl(input, { ...init, headers });
    };

    let response = await attempt(false);
    const explicitlyNotDispatched =
      response.headers.get('X-Arete-Upstream-Attempted') === 'false';
    if (
      !response.ok
      && authentication.manager
      && refreshableErrorCode(response)
      && (!requirePreDispatchMarker || explicitlyNotDispatched)
    ) {
      authentication.manager.clearHttpAuthToken(request);
      response = await attempt(true);
    }
    return response;
  };

  const chainAuthentication = bindingAuthentication(bindings.chain);
  const transactionAuthentication = bindingAuthentication(bindings.transactions);
  const chainFetch = authenticatedFetch(chainAuthentication);
  const transactionFetch = authenticatedFetch(transactionAuthentication);

  return {
    chain: createChainClient(
      bindings.chain.endpoint,
      ((input: RequestInfo | URL, init?: RequestInit) => chainFetch(
        typeof input === 'string'
          ? input
          : input instanceof URL ? input.toString() : input.url,
        init,
        'read',
        false
      )) as typeof fetch
    ),
    transactions: createTransactionTransport(
      bindings.transactions.endpoint,
      (input, init, scope: TransactionAuthScope, requirePreDispatchMarker) =>
        transactionFetch(input, init, scope, requirePreDispatchMarker)
    ),
  };
}
