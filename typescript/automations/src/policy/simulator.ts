/**
 * A Solana JSON-RPC endpoint the policy simulates prepared transactions on.
 *
 * The summary calls two methods: `getMultipleAccounts` (lookup tables and
 * the pre-state of written accounts) and `simulateTransaction` (with the
 * written accounts' post-state and inner instructions). `call` resolves
 * with the JSON-RPC `result` and rejects on a JSON-RPC `error`.
 */
export interface Simulator {
  call(method: string, params: readonly unknown[]): Promise<unknown>;
}

import { DevEnvironmentError, type EnvironmentOption, resolveEnvironmentSwitch } from '@usearete/sdk';

export interface RpcSimulatorOptions {
  /**
   * Without a URL: dev mode (this option, else `ARETE_ENVIRONMENT=dev`)
   * simulates on the dev environment through the `a4 dev up` edge
   * (`ARETE_DEV_URL`, or the running session's). Production needs a URL.
   */
  readonly environment?: EnvironmentOption;
  /** Defaults to the global `fetch`. */
  readonly fetch?: typeof fetch;
  /** Extra request headers, for example an RPC provider's API key. */
  readonly headers?: Readonly<Record<string, string>>;
  /**
   * How long one call may take, request and response body together, in
   * milliseconds. Defaults to 15 seconds. A call that runs out fails, so the
   * transaction's effect is unknown and the policy refuses it.
   */
  readonly timeoutMs?: number;
}

/**
 * A {@link Simulator} over a JSON-RPC URL. With no URL, dev mode uses the
 * dev environment (its fork's state) through the `a4 dev up` edge;
 * production has no default.
 */
export function rpcSimulator(url?: string, options: RpcSimulatorOptions = {}): Simulator {
  url ??= devSimulatorUrl(options.environment);
  const fetchImpl = options.fetch ?? globalThis.fetch;
  if (typeof fetchImpl !== 'function') {
    throw new TypeError('rpcSimulator needs a fetch implementation');
  }
  const timeoutMs = options.timeoutMs ?? 15_000;
  return {
    async call(method, params) {
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), timeoutMs);
      try {
        let response: Response;
        try {
          response = await fetchImpl(url, {
            method: 'POST',
            headers: { 'content-type': 'application/json', ...options.headers },
            body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
            signal: controller.signal,
          });
        } catch {
          throw new Error(
            controller.signal.aborted
              ? `The RPC did not answer ${method} within ${timeoutMs}ms`
              : `The RPC is unreachable for ${method}`,
          );
        }
        let body: unknown;
        try {
          body = await response.json();
        } catch {
          throw new Error(
            controller.signal.aborted
              ? `The RPC did not answer ${method} within ${timeoutMs}ms`
              : `The RPC answered ${method} with invalid JSON`,
          );
        }
        const record = (body ?? {}) as { error?: { message?: unknown }; result?: unknown };
        if (record.error !== undefined && record.error !== null) {
          const message =
            typeof record.error.message === 'string' ? record.error.message : 'unknown error';
          throw new Error(`${method} failed: ${message}`);
        }
        return record.result ?? null;
      } finally {
        clearTimeout(timer);
      }
    },
  };
}

/** The edge's JSON-RPC, which serves the dev environment's fork. */
function devSimulatorUrl(option: EnvironmentOption | undefined): string {
  const environment = resolveEnvironmentSwitch(option);
  if (environment.mode === 'production') {
    throw new TypeError(
      'rpcSimulator needs a JSON-RPC URL in production; only dev mode simulates against the dev environment by default',
    );
  }
  if (environment.via === 'direct') {
    throw new DevEnvironmentError(
      'devenv-surface-unavailable',
      'Direct dev mode has no JSON-RPC endpoint to simulate on',
      { fix: 'Run through the `a4 dev up` edge (`a4 dev run -- <command>`), or pass rpcSimulator a JSON-RPC URL.' },
    );
  }
  return environment.edgeUrl;
}
