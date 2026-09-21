/** Shared Accept negotiation and agent-facing error bodies.
 *  Used by Vercel Edge middleware and the local AFDocs score server.
 */

export const SITE = "https://docs.arete.run";

const MARKDOWN_TYPE = "text/markdown; charset=utf-8";
const JSON_TYPE = "application/json; charset=utf-8";

export function prefersMarkdown(accept) {
  return /\btext\/markdown\b/i.test(accept ?? "");
}

/** JSON without HTML, so browsers still get the HTML docs page. */
export function prefersJson(accept) {
  const a = accept ?? "";
  return /\bapplication\/json\b/i.test(a) && !/\btext\/html\b/i.test(a);
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
