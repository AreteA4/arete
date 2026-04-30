import type { APIRoute } from "astro";
import { getEntry } from "astro:content";

// Serves the raw markdown source of the homepage at /index.md.
// Same purpose as [...slug].md.ts but the catch-all can't match the empty slug.

export const GET: APIRoute = async () => {
  const entry = await getEntry("docs", "index");
  if (!entry) {
    return new Response("Not Found", { status: 404 });
  }
  const fm = Object.entries(entry.data)
    .map(([k, v]) => `${k}: ${JSON.stringify(v)}`)
    .join("\n");
  const body = `---\n${fm}\n---\n\n${entry.body ?? ""}`;
  return new Response(body, {
    headers: {
      "content-type": "text/markdown; charset=utf-8",
      "cache-control": "public, max-age=0, must-revalidate",
    },
  });
};
