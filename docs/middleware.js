/**
 * Vercel Edge Middleware.
 *
 * Static HTML on Vercel is served before `vercel.json` rewrites, so
 * `Accept: text/markdown` never reaches the Astro `.md` endpoints in
 * production. Middleware runs before the static file, so agents that
 * send that Accept header get the markdown twin at the same URL.
 *
 * Missing pages keep HTTP 404. Markdown and JSON Accept headers get an
 * agent-parseable error body; HTML keeps the existing Starlight 404 page.
 *
 * Extensionless `/.well-known/*` aliases (agent-skills, mcp) are JSON
 * discovery documents with no `.md` twin. Leave them for `vercel.json`
 * rewrites, matching the local score server's dotted-path skip.
 */

import {
  isDiscoveryPath,
  markdownTwinPath,
  markdown404,
  json404,
  prefersJson,
  prefersMarkdown,
  toFetchResponse,
  withMarkdownHeaders,
} from "./lib/agent-http.mjs";

export const config = {
  matcher: [
    "/((?!_astro|api|ingest|a4/|openapi\\.json|\\.well-known|.*\\.(?:css|js|mjs|map|svg|png|jpg|jpeg|gif|webp|json|xml|ico|md|txt|woff|woff2|sh|ps1)$).*)",
  ],
};

export default async function middleware(request) {
  const accept = request.headers.get("accept") ?? "";
  const url = new URL(request.url);
  const pathname = url.pathname;

  if (isDiscoveryPath(pathname)) {
    return;
  }

  if (prefersMarkdown(accept)) {
    const mdUrl = new URL(request.url);
    mdUrl.pathname = markdownTwinPath(pathname);
    const res = await fetch(mdUrl, request);
    if (res.ok) {
      return withMarkdownHeaders(res);
    }
    return toFetchResponse(markdown404(pathname));
  }

  if (prefersJson(accept)) {
    const mdUrl = new URL(request.url);
    mdUrl.pathname = markdownTwinPath(pathname);
    const probe = await fetch(mdUrl, {
      method: "GET",
      headers: { accept: "text/markdown" },
    });
    if (probe.ok) {
      return;
    }
    return toFetchResponse(json404(pathname));
  }
}
