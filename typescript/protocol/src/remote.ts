/**
 * The server's release check: the expression and template checks an upload
 * runs, on a release body, without storing it (`POST /api/protocols/check`).
 * The server is the reference for protocol validity; the local checker in
 * `check/` covers syntax and names only.
 */

export const DEFAULT_API_URL = "https://api.arete.run";

/** Where a remote check goes, and the secret key it sends. */
export interface RemoteCheckTarget {
  apiUrl: string;
  apiKey: string;
}

/** One rejection: a JSON pointer into the spec, a stable code and a message. */
export interface RemoteCheckError {
  path?: string;
  code?: string;
  message: string;
}

export type RemoteCheckOutcome =
  | {
      kind: "passed";
      releaseHash: string;
      /** `pending` while a pinned program spec is not on the server yet. */
      templateCheck: "passed" | "pending";
      missingProgramSpecs: Array<{ alias?: string; programSpecHash?: string }>;
    }
  | { kind: "rejected"; status: number; code?: string; error: string; errors: RemoteCheckError[] }
  /**
   * The server did not check the release: it has no check route, gave an
   * answer that is not a check result, or did not answer in time.
   */
  | { kind: "unavailable"; reason: string };

/**
 * The remote check target from the environment: `ARETE_API_KEY` (a secret
 * key) and `ARETE_API_URL` (default {@link DEFAULT_API_URL}). Returns a
 * reason instead when there is no usable key.
 */
export function remoteCheckTarget(
  env: Record<string, string | undefined> = process.env,
): RemoteCheckTarget | { reason: string } {
  const apiKey = env.ARETE_API_KEY?.trim();
  if (!apiKey) return { reason: "no ARETE_API_KEY" };
  if (apiKey.startsWith("a4_pk_")) return { reason: "ARETE_API_KEY is a publishable key; the check needs a secret key" };
  const apiUrl = (env.ARETE_API_URL?.trim() || DEFAULT_API_URL).replace(/\/+$/, "").replace(/\/api$/, "");
  return { apiUrl, apiKey };
}

function errorList(problem: Record<string, unknown>): RemoteCheckError[] {
  const nested = (problem.templateCheck as { errors?: unknown } | undefined)?.errors;
  const list = Array.isArray(problem.errors) ? problem.errors : Array.isArray(nested) ? nested : [];
  return list.map((entry): RemoteCheckError => {
    if (typeof entry === "string") return { message: entry };
    const item = (entry ?? {}) as Record<string, unknown>;
    return {
      ...(typeof item.path === "string" ? { path: item.path } : {}),
      ...(typeof item.code === "string" ? { code: item.code } : {}),
      message: typeof item.message === "string" ? item.message : JSON.stringify(entry),
    };
  });
}

/** How long a remote check may take, request and response body together. */
export const REMOTE_CHECK_TIMEOUT_MS = 30_000;

/** Send `body` (the upload body) to the server's release check. */
export async function checkRemotely(
  body: { spec: unknown; bundleBase64?: string },
  target: RemoteCheckTarget,
  fetchImpl: typeof fetch = fetch,
  timeoutMs: number = REMOTE_CHECK_TIMEOUT_MS,
): Promise<RemoteCheckOutcome> {
  const url = `${target.apiUrl}/api/protocols/check`;
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  let response: Response;
  let text: string;
  try {
    response = await fetchImpl(url, {
      method: "POST",
      headers: { authorization: `Bearer ${target.apiKey}`, "content-type": "application/json" },
      body: JSON.stringify(body),
      signal: controller.signal,
    });
    text = await response.text();
  } catch (error) {
    if (controller.signal.aborted) {
      return { kind: "unavailable", reason: `the release check at ${url} did not answer within ${timeoutMs / 1000}s` };
    }
    throw error;
  } finally {
    clearTimeout(timer);
  }
  let parsed: unknown;
  let json = true;
  try {
    parsed = text === "" ? {} : JSON.parse(text);
  } catch {
    json = false;
    parsed = { error: text };
  }
  const problem = (parsed !== null && typeof parsed === "object" ? parsed : {}) as Record<string, unknown>;
  if (response.ok) {
    const { releaseHash, templateCheck } = problem;
    if (
      !json ||
      typeof releaseHash !== "string" ||
      releaseHash === "" ||
      (templateCheck !== "passed" && templateCheck !== "pending")
    ) {
      return {
        kind: "unavailable",
        reason: `unexpected answer from ${url} (HTTP ${response.status}, no release hash and template check status); check ARETE_API_URL`,
      };
    }
    return {
      kind: "passed",
      releaseHash,
      templateCheck,
      missingProgramSpecs: Array.isArray(problem.missingProgramSpecs)
        ? (problem.missingProgramSpecs as Array<{ alias?: string; programSpecHash?: string }>)
        : [],
    };
  }
  const code = typeof problem.code === "string" ? problem.code : undefined;
  if (response.status === 404 && code === undefined) {
    return { kind: "unavailable", reason: `${target.apiUrl} has no release check route` };
  }
  return {
    kind: "rejected",
    status: response.status,
    ...(code !== undefined ? { code } : {}),
    error: typeof problem.error === "string" ? problem.error : `HTTP ${response.status}`,
    errors: errorList(problem),
  };
}
