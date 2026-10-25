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
      templateCheck: string;
      missingProgramSpecs: Array<{ alias?: string; programSpecHash?: string }>;
    }
  | { kind: "rejected"; status: number; code?: string; error: string; errors: RemoteCheckError[] }
  /** The server has no check route; only an upload runs its checks. */
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

/** Send `body` (the upload body) to the server's release check. */
export async function checkRemotely(
  body: { spec: unknown; bundleBase64?: string },
  target: RemoteCheckTarget,
  fetchImpl: typeof fetch = fetch,
): Promise<RemoteCheckOutcome> {
  const response = await fetchImpl(`${target.apiUrl}/api/protocols/check`, {
    method: "POST",
    headers: { authorization: `Bearer ${target.apiKey}`, "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  const text = await response.text();
  let parsed: unknown;
  try {
    parsed = text === "" ? {} : JSON.parse(text);
  } catch {
    parsed = { error: text };
  }
  const problem = (parsed ?? {}) as Record<string, unknown>;
  if (response.ok) {
    return {
      kind: "passed",
      releaseHash: String(problem.releaseHash ?? ""),
      templateCheck: String(problem.templateCheck ?? "passed"),
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
