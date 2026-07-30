import { ConnectionManager } from './connection';
import {
  AreteError,
  parseErrorCode,
  shouldRefreshToken,
  type AuthConfig,
  type ProgramReadBinding,
  type ProgramReadBindingAuthTarget,
  type ProgramReleaseReference,
} from './types';
import { ReadRequestError } from './read';

export type ProgramReadRequest =
  | {
      readonly operation: 'fetch';
      readonly account: string;
      readonly address: string;
    }
  | {
      readonly operation: 'fetchMany';
      readonly account: string;
      readonly addresses: readonly string[];
    }
  | {
      readonly operation: 'exists';
      readonly account: string;
      readonly address: string;
    };

export interface ProgramReadTransport {
  read<T>(request: ProgramReadRequest): Promise<T>;
}

type ProgramReadTransportConfig =
  | {
      readonly kind: 'local-http';
      readonly endpoint: string;
      readonly release: ProgramReleaseReference;
      readonly fetch: typeof fetch;
    }
  | {
      readonly kind: 'hosted-binding';
      readonly release: ProgramReleaseReference;
      readonly binding: ProgramReadBinding;
      readonly auth?: AuthConfig;
      readonly fetch: typeof fetch;
    }
  | {
      readonly kind: 'unavailable';
      readonly message: string;
    };

function appendUrl(baseUrl: string, path: string): string {
  return `${baseUrl.replace(/\/+$/, '')}/${path.replace(/^\/+/, '')}`;
}

function responseErrorCode(response: Response, body: string): string | undefined {
  const headerCode = response.headers.get('X-Error-Code');
  if (headerCode) return headerCode;
  try {
    const parsed = JSON.parse(body) as {
      code?: unknown;
      error?: { code?: unknown };
    };
    if (typeof parsed.error?.code === 'string') return parsed.error.code;
    return typeof parsed.code === 'string' ? parsed.code : undefined;
  } catch {
    return undefined;
  }
}

function isRefreshableErrorCode(code: string): boolean {
  return shouldRefreshToken(parseErrorCode(code.trim().replace(/_/g, '-')));
}

function parseProgramReadResponse<T>(response: Response, path: string, body: string): T {
  if (!response.ok) {
    throw new ReadRequestError({
      status: response.status,
      path,
      body,
      serverErrorCode: responseErrorCode(response, body),
    });
  }

  try {
    return JSON.parse(body) as T;
  } catch (error) {
    throw new AreteError(
      `Program read '${path}' returned invalid JSON`,
      'INVALID_RESPONSE',
      error
    );
  }
}

function hostedAuthConfig(
  binding: ProgramReadBinding,
  runtimeAuth: AuthConfig | undefined
): AuthConfig | undefined {
  const metadata = binding.auth;
  const runtimeStrategyConfigured = runtimeAuth?.token !== undefined
    || runtimeAuth?.getToken !== undefined
    || runtimeAuth?.tokenEndpoint !== undefined;
  if (runtimeStrategyConfigured) return runtimeAuth;
  if (metadata.required === false) return runtimeAuth;
  return { ...runtimeAuth, tokenEndpoint: metadata.sessionEndpoint };
}

function hostedAuth(
  config: Extract<ProgramReadTransportConfig, { kind: 'hosted-binding' }>
): {
  readonly staticToken?: string;
  readonly manager?: ConnectionManager;
  readonly target?: ProgramReadBindingAuthTarget;
} {
  const auth = hostedAuthConfig(config.binding, config.auth);
  if (auth?.token !== undefined && !auth.token) {
    throw new AreteError('Authentication token is empty', 'TOKEN_INVALID');
  }

  const bindingId = config.binding.programReadBindingId;
  if (!auth || auth.token !== undefined) {
    return { staticToken: auth?.token };
  }

  return {
    manager: new ConnectionManager({ websocketUrl: null, auth, fetch: config.fetch }),
    target: {
      targetKind: 'program-read-binding',
      targetId: bindingId,
      programReleaseHash: config.release.programReleaseHash,
    },
  };
}

function requestPath(
  release: ProgramReleaseReference,
  request: ProgramReadRequest
): string {
  const releaseHash = encodeURIComponent(release.programReleaseHash).replace(/%3A/g, ':');
  const root = `/v1/releases/${releaseHash}`
    + `/accounts/${encodeURIComponent(request.account)}`;
  if (request.operation === 'fetchMany') return root;
  const addressPath = `${root.replace(/\/+$/, '')}/${encodeURIComponent(request.address)}`;
  return request.operation === 'exists' ? `${addressPath}/exists` : addressPath;
}

export function createProgramReadTransport(
  config: ProgramReadTransportConfig
): ProgramReadTransport {
  if (config.kind === 'unavailable') {
    return {
      async read<T>(): Promise<T> {
        throw new AreteError(config.message, 'INVALID_CONFIG');
      },
    };
  }

  const auth = config.kind === 'hosted-binding' ? hostedAuth(config) : undefined;
  const endpoint = config.kind === 'hosted-binding'
    ? config.binding.endpoint
    : config.endpoint;

  return {
    async read<T>(request: ProgramReadRequest): Promise<T> {
      const path = requestPath(config.release, request);
      const input = appendUrl(endpoint, path);
      const attempt = async (forceRefresh: boolean): Promise<Response> => {
        const token = auth?.staticToken
          ?? (auth?.manager && auth.target
            ? await auth.manager.getHttpAuthToken(auth.target, ['read'], forceRefresh)
            : undefined);
        const headers = new Headers(
          request.operation === 'fetchMany'
            ? { 'content-type': 'application/json' }
            : undefined
        );
        if (token) headers.set('authorization', `Bearer ${token}`);
        return config.fetch(input, {
          method: request.operation === 'fetchMany' ? 'POST' : 'GET',
          headers,
          body: request.operation === 'fetchMany'
            ? JSON.stringify({ addresses: request.addresses })
            : undefined,
        });
      };

      let response = await attempt(false);
      let body = await response.text();
      const wireCode = responseErrorCode(response, body);
      if (
        response.status === 401
        && wireCode
        && isRefreshableErrorCode(wireCode)
        && auth?.manager
        && auth.target
      ) {
        auth.manager.clearHttpAuthToken(auth.target);
        response = await attempt(true);
        body = await response.text();
      }
      return parseProgramReadResponse<T>(response, path, body);
    },
  };
}
