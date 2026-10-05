export interface FetchStubRequest {
  readonly url: string;
  readonly method: string;
  readonly headers: Headers;
  /** Request body as text, when one was sent. */
  readonly body: string | undefined;
}

/** A route's answer: a `Response`, or a value serialized as JSON with status 200. */
export type FetchStubReply = Response | unknown;

export interface FetchStubRoute {
  /** Substring of the URL, a pattern, or a predicate. */
  readonly match: string | RegExp | ((request: FetchStubRequest) => boolean);
  /** Only match this HTTP method (case-insensitive). */
  readonly method?: string;
  /** Static reply, or a function computing one per request. */
  readonly reply: FetchStubReply | ((request: FetchStubRequest) => FetchStubReply | Promise<FetchStubReply>);
  /** Status for non-`Response` replies (default 200). */
  readonly status?: number;
}

export interface FetchStubOptions {
  /** Answer for requests no route matches (default: a 404 JSON error). */
  readonly fallback?: (request: FetchStubRequest) => FetchStubReply | Promise<FetchStubReply>;
}

export interface FetchStub {
  (input: RequestInfo | URL, init?: RequestInit): Promise<Response>;
  /** Every request, in order. */
  readonly calls: FetchStubRequest[];
  /** Forget every recorded request. */
  reset(): void;
}

/**
 * JSON with the SDK's wire rule for integers: `bigint` values are written as
 * decimal strings, as the server sends u64 fields.
 */
export function jsonResponse(body: unknown, init: ResponseInit = {}): Response {
  const headers = new Headers(init.headers);
  if (!headers.has('content-type')) headers.set('content-type', 'application/json');
  return new Response(
    JSON.stringify(body, (_key, value) => (typeof value === 'bigint' ? value.toString(10) : value)),
    { ...init, status: init.status ?? 200, headers },
  );
}

function matches(route: FetchStubRoute, request: FetchStubRequest): boolean {
  if (route.method && route.method.toUpperCase() !== request.method) return false;
  if (typeof route.match === 'string') return request.url.includes(route.match);
  if (route.match instanceof RegExp) return route.match.test(request.url);
  return route.match(request);
}

async function toRequest(input: RequestInfo | URL, init?: RequestInit): Promise<FetchStubRequest> {
  if (typeof Request !== 'undefined' && input instanceof Request) {
    const body = init?.body ?? (input.body ? await input.clone().text() : undefined);
    return {
      url: input.url,
      method: (init?.method ?? input.method).toUpperCase(),
      headers: new Headers(init?.headers ?? input.headers),
      body: typeof body === 'string' ? body : body === undefined || body === null ? undefined : String(body),
    };
  }
  const body = init?.body;
  return {
    url: input instanceof URL ? input.toString() : String(input),
    method: (init?.method ?? 'GET').toUpperCase(),
    headers: new Headers(init?.headers),
    body: typeof body === 'string' ? body : body === undefined || body === null ? undefined : String(body),
  };
}

/**
 * A routed, recording `fetch` for tests: pass it as `ConnectOptions.fetch`,
 * `createSession(…, { fetch })` or `<AreteProvider fetch={…}>`. Routes are
 * tried in order; the first match answers.
 *
 * ```ts
 * const fetch = createFetchStub([
 *   { match: '/accounts/Miner/', reply: { authority: 'wallet', rewards_ore: '12' } },
 *   { match: '/chain/exists/', reply: { exists: true } },
 * ]);
 * ```
 */
export function createFetchStub(
  routes: readonly FetchStubRoute[] = [],
  options: FetchStubOptions = {},
): FetchStub {
  const calls: FetchStubRequest[] = [];
  const stub = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const request = await toRequest(input, init);
    calls.push(request);
    const route = routes.find((candidate) => matches(candidate, request));
    const reply = route
      ? typeof route.reply === 'function'
        ? await (route.reply as (request: FetchStubRequest) => FetchStubReply)(request)
        : route.reply
      : options.fallback
        ? await options.fallback(request)
        : jsonResponse(
            { code: 'not-found', message: `No fetch stub route for ${request.method} ${request.url}` },
            { status: 404 },
          );
    return reply instanceof Response
      ? reply
      : jsonResponse(reply, { status: route?.status ?? 200 });
  };
  return Object.assign(stub, {
    calls,
    reset() {
      calls.length = 0;
    },
  });
}
