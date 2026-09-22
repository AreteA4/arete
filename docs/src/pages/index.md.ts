import type { APIRoute } from "astro";
import { getEntry } from "astro:content";
import { markdownResponse, renderMarkdownPage } from "../lib/markdown-page";

// Serves the raw markdown source of the homepage at /index.md.
// Same purpose as [...slug].md.ts but the catch-all can't match the empty slug.

export const GET: APIRoute = async () => {
  const entry = await getEntry("docs", "index");
  if (!entry) {
    return new Response("Not Found", { status: 404 });
  }
  return markdownResponse(
    renderMarkdownPage(
      { body: entry.body ?? "", data: entry.data as Record<string, unknown> },
      "",
    ),
  );
};
