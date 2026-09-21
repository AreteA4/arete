import { stripMdx } from "./strip-mdx";

const SITE = "https://docs.arete.run";

export function markdownSlug(entryId: string): string {
  const slug = entryId.replace(/\.(md|mdx)$/, "");
  return slug === "index" ? "" : slug;
}

export function markdownPath(slug: string): string {
  return slug ? `/${slug}.md` : "/index.md";
}

/** Mintlify-style first-hop directive prepended to every per-page markdown twin. */
export function markdownDirective(slug: string): string {
  const path = markdownPath(slug);
  return [
    `> For the complete documentation index optimized for AI agents, see [llms.txt](${SITE}/llms.txt) or [llms-full.txt](${SITE}/llms-full.txt). A markdown version of this page is available at [${path}](${SITE}${path}) or by sending \`Accept: text/markdown\`.`,
    "",
  ].join("\n");
}

export function renderMarkdownPage(
  entry: { body?: string; data: Record<string, unknown> },
  slug: string,
): string {
  const fm = Object.entries(entry.data)
    .map(([k, v]) => `${k}: ${JSON.stringify(v)}`)
    .join("\n");
  const body = stripMdx(entry.body ?? "");
  return `---\n${fm}\n---\n\n${markdownDirective(slug)}${body}\n`;
}

export function markdownResponse(body: string): Response {
  return new Response(body, {
    headers: {
      "content-type": "text/markdown; charset=utf-8",
      "cache-control": "public, max-age=0, must-revalidate",
      "x-llms-txt": `${SITE}/llms.txt`,
    },
  });
}
