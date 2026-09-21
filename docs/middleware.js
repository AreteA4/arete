/**
 * Vercel Edge Middleware.
 *
 * Static HTML on Vercel is served before `vercel.json` rewrites, so
 * `Accept: text/markdown` never reaches the Astro `.md` endpoints in
 * production. Middleware runs before the static file, so agents that
 * send that Accept header get the markdown twin at the same URL.
 */

export const config = {
  matcher: [
    "/((?!_astro|api|ingest|a4/|.*\\.(?:css|js|mjs|map|svg|png|jpg|jpeg|gif|webp|json|xml|ico|md|txt|woff|woff2|sh|ps1)$).*)",
  ],
};

export default function middleware(request) {
  const accept = request.headers.get("accept") ?? "";
  if (!/\btext\/markdown\b/i.test(accept)) {
    return;
  }

  const url = new URL(request.url);
  const pathname = url.pathname;
  if (pathname.endsWith(".md") || pathname.endsWith(".txt")) {
    return;
  }

  url.pathname =
    pathname === "/" ? "/index.md" : `${pathname.replace(/\/$/, "")}.md`;
  return fetch(url, request);
}
