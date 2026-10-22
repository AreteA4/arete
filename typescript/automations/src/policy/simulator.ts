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

export interface RpcSimulatorOptions {
  /** Defaults to the global `fetch`. */
  readonly fetch?: typeof fetch;
  /** Extra request headers, for example an RPC provider's API key. */
  readonly headers?: Readonly<Record<string, string>>;
}

/** A {@link Simulator} over a JSON-RPC URL. */
export function rpcSimulator(url: string, options: RpcSimulatorOptions = {}): Simulator {
  const fetchImpl = options.fetch ?? globalThis.fetch;
  if (typeof fetchImpl !== 'function') {
    throw new TypeError('rpcSimulator needs a fetch implementation');
  }
  return {
    async call(method, params) {
      let response: Response;
      try {
        response = await fetchImpl(url, {
          method: 'POST',
          headers: { 'content-type': 'application/json', ...options.headers },
          body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
        });
      } catch {
        throw new Error(`The RPC is unreachable for ${method}`);
      }
      let body: unknown;
      try {
        body = await response.json();
      } catch {
        throw new Error(`The RPC answered ${method} with invalid JSON`);
      }
      const record = (body ?? {}) as { error?: { message?: unknown }; result?: unknown };
      if (record.error !== undefined && record.error !== null) {
        const message =
          typeof record.error.message === 'string' ? record.error.message : 'unknown error';
        throw new Error(`${method} failed: ${message}`);
      }
      return record.result ?? null;
    },
  };
}
