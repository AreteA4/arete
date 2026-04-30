import type { APIRoute, GetStaticPaths } from "astro";
import { getCollection } from "astro:content";

// Serves the raw markdown source of every doc page at <path>.md.
// Pairs with the Vercel rewrite that maps `Accept: text/markdown` requests
// to these routes, so agents can fetch markdown without HTML conversion.

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
  const body = `---\n${fm}\n---\n\n${entry.body ?? ""}`;
  return new Response(body, {
    headers: {
      "content-type": "text/markdown; charset=utf-8",
      "cache-control": "public, max-age=0, must-revalidate",
    },
  });
};
