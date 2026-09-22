/** Shared Accept negotiation and agent-facing error bodies.
 *  Used by Vercel Edge middleware and the local AFDocs score server.
 */

export const SITE = "https://docs.arete.run";

const MARKDOWN_TYPE = "text/markdown; charset=utf-8";
const JSON_TYPE = "application/json; charset=utf-8";

function parseAccept(accept) {
  if (!accept) return [];
  return accept.split(",").flatMap((part) => {
    const pieces = part
      .trim()
      .split(";")
      .map((item) => item.trim())
      .filter(Boolean);
    if (!pieces.length) return [];
    const range = pieces[0].toLowerCase();
    let q = 1;
    for (const param of pieces.slice(1)) {
      const eq = param.indexOf("=");
      if (eq === -1) continue;
      if (param.slice(0, eq).trim().toLowerCase() !== "q") continue;
      const value = Number(param.slice(eq + 1).trim());
      if (Number.isFinite(value)) q = Math.min(1, Math.max(0, value));
    }
    return [{ range, q }];
  });
}

/** Quality for an exact type. Ignores star-slash-star so browsers stay on HTML. */
export function mediaQuality(accept, type) {
  const wanted = type.toLowerCase();
  let best = 0;
  let matched = false;
  for (const { range, q } of parseAccept(accept)) {
    if (range === wanted) {
      matched = true;
      if (q > best) best = q;
    }
  }
  return matched ? best : 0;
}

export function prefersMarkdown(accept) {
  return mediaQuality(accept, "text/markdown") > 0;
}

/** JSON without HTML, so browsers still get the HTML docs page. */
export function prefersJson(accept) {
  return (
    mediaQuality(accept, "application/json") > 0 &&
    mediaQuality(accept, "text/html") === 0
  );
}

export function isDiscoveryPath(pathname) {
  return (
    pathname.startsWith("/.well-known/") ||
    pathname.startsWith("/api/") ||
    pathname.startsWith("/mcp") ||
    pathname === "/openapi.json" ||
    pathname.endsWith(".md") ||
    pathname.endsWith(".txt") ||
    pathname.endsWith(".json")
  );
}

export function markdownTwinPath(pathname) {
  return pathname === "/" ? "/index.md" : `${pathname.replace(/\/$/, "")}.md`;
}

export function markdownHeaders(extra = {}) {
  return {
    "content-type": MARKDOWN_TYPE,
    "cache-control": "public, max-age=0, must-revalidate",
    vary: "Accept",
    "x-llms-txt": `${SITE}/llms.txt`,
    ...extra,
  };
}

export function jsonHeaders(extra = {}) {
  return {
    "content-type": JSON_TYPE,
    "cache-control": "public, max-age=0, must-revalidate",
    vary: "Accept",
    "x-content-type-options": "nosniff",
    "access-control-allow-origin": "*",
    ...extra,
  };
}

export function markdown404Body(pathname) {
  const path = pathname || "/";
  return [
    "# Page not found",
    "",
    `No documentation page exists at \`${path}\` on docs.arete.run.`,
    "",
    "This is an HTTP 404. Use the documentation index to recover:",
    "",
    `- [llms.txt](${SITE}/llms.txt) — page map of every markdown twin`,
    `- [Sitemap](${SITE}/sitemap.xml) — HTML page list`,
    `- [Arete docs home](${SITE}/) — start here`,
    `- [Documentation MCP](${SITE}/mcp) — search_docs and fetch_page`,
    "",
    "Append `.md` to any real docs URL, or send `Accept: text/markdown`, to read the markdown twin.",
    "",
  ].join("\n");
}

export function jsonErrorBody(code, message, hint) {
  return {
    error: {
      code,
      message,
      hint,
    },
  };
}

export function json404Body(pathname) {
  return jsonErrorBody(
    "not_found",
    `No documentation page or API route exists at ${pathname || "/"}.`,
    `Read ${SITE}/llms.txt or ${SITE}/openapi.json, or call search_docs on ${SITE}/mcp.`,
  );
}

export function markdown404(pathname) {
  return {
    status: 404,
    headers: markdownHeaders(),
    body: markdown404Body(pathname),
  };
}

export function json404(pathname) {
  return {
    status: 404,
    headers: jsonHeaders(),
    body: JSON.stringify(json404Body(pathname), null, 2),
  };
}

export function toFetchResponse({ status, headers, body }) {
  return new Response(body, { status, headers });
}

export function withMarkdownHeaders(res) {
  const headers = new Headers(res.headers);
  headers.set("content-type", MARKDOWN_TYPE);
  headers.set("vary", "Accept");
  headers.set("x-llms-txt", `${SITE}/llms.txt`);
  return new Response(res.body, { status: res.status, headers });
}

/** Vercel Routing Middleware rewrite: serve the static twin from cache. */
export function rewriteTo(url) {
  return new Response(null, {
    headers: {
      "x-middleware-rewrite": url.toString(),
    },
  });
}
