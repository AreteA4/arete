import type { APIRoute, GetStaticPaths } from "astro";
import { getCollection } from "astro:content";
import {
  markdownResponse,
  markdownSlug,
  renderMarkdownPage,
} from "../lib/markdown-page";

// Serves the raw markdown source of every doc page at <path>.md.
// Pairs with Vercel Edge middleware that maps `Accept: text/markdown`
// requests to these routes, so agents can fetch markdown without HTML
// conversion. MDX-only syntax (imports, JSX tags) is stripped so the
// output is valid CommonMark. A Mintlify-style llms.txt blockquote is
// prepended so converted or fetched markdown has a first-hop index.

export const getStaticPaths: GetStaticPaths = async () => {
  const docs = await getCollection("docs");
  return docs
    .filter((entry) => !["index", "index.md", "index.mdx"].includes(entry.id))
    .map((entry) => {
      const slug = entry.id.replace(/\.(md|mdx)$/, "");
      return {
        params: { slug },
        props: { entry },
      };
    });
};

export const GET: APIRoute = ({ props }) => {
  const { entry } = props as {
    entry: {
      id: string;
      body: string;
      data: Record<string, unknown>;
    };
  };
  const slug = markdownSlug(entry.id);
  return markdownResponse(renderMarkdownPage(entry, slug));
};
