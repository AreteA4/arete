import type { APIRoute, GetStaticPaths } from "astro";
import { getCollection } from "astro:content";
import { stripMdx } from "../lib/strip-mdx";

// Serves the raw markdown source of every doc page at <path>.md.
// Pairs with the Vercel rewrite that maps `Accept: text/markdown` requests
// to these routes, so agents can fetch markdown without HTML conversion.
// MDX-only syntax (imports, JSX tags) is stripped so the output is valid
// CommonMark for consumers that can't parse MDX.

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
  const fm = Object.entries(entry.data)
    .map(([k, v]) => `${k}: ${JSON.stringify(v)}`)
    .join("\n");
  const body = `---\n${fm}\n---\n\n${stripMdx(entry.body ?? "")}\n`;
  return new Response(body, {
    headers: {
      "content-type": "text/markdown; charset=utf-8",
      "cache-control": "public, max-age=0, must-revalidate",
    },
  });
};
