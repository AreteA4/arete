// MCP server for Arete documentation, deployed as a Vercel serverless function.
// Vercel auto-detects api/*.ts files and runs them as Node functions independently
// of the static Astro build, so we don't need to switch Astro to hybrid/server mode.
//
// Endpoint: https://docs.arete.run/api/mcp/sse  (also reachable at /mcp/sse via
// the rewrite in vercel.json).
//
// Tools exposed:
//   - search_docs(query): keyword search across llms-full.txt
//   - fetch_page(slug):   returns raw markdown for a single doc page
//
// Discovery manifest lives at /.well-known/mcp.json.

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/streamableHttp.js";
import { z } from "zod";

const DOCS_BASE = "https://docs.arete.run";

function buildServer(): McpServer {
  const server = new McpServer({
    name: "arete-docs",
    version: "1.0.0",
  });

  server.tool(
    "search_docs",
    "Search the Arete documentation. Returns matching page snippets ranked by relevance. Use this when answering questions about Arete features, the Rust DSL, SDKs, or CLI.",
    {
      query: z.string().describe("Search query — keywords or a full question"),
      limit: z
        .number()
        .int()
        .min(1)
        .max(20)
        .default(5)
        .describe("Max number of results to return"),
    },
    async ({ query, limit }) => {
      const res = await fetch(`${DOCS_BASE}/llms-full.txt`);
      if (!res.ok) {
        return {
          content: [
            { type: "text", text: `Failed to load docs index: ${res.status}` },
          ],
          isError: true,
        };
      }
      const fullText = await res.text();
      // starlight-llms-txt separates pages with horizontal rules
      const pages = fullText.split(/^\* \* \*$|^---$/m);
      const q = query.toLowerCase();
      const scored = pages
        .map((page) => {
          const lower = page.toLowerCase();
          const hits = (lower.match(new RegExp(escapeRegex(q), "g")) ?? []).length;
          const titleMatch = /^# (.+)/m.exec(page);
          return {
            title: titleMatch?.[1]?.trim() ?? "(untitled)",
            snippet: page.slice(0, 600).trim(),
            hits,
          };
        })
        .filter((p) => p.hits > 0)
        .sort((a, b) => b.hits - a.hits)
        .slice(0, limit);

      if (scored.length === 0) {
        return {
          content: [
            {
              type: "text",
              text: `No matches for "${query}". Try broader keywords.`,
            },
          ],
        };
      }
      const text = scored
        .map(
          (p, i) =>
            `## Result ${i + 1}: ${p.title} (${p.hits} hits)\n\n${p.snippet}`,
        )
        .join("\n\n---\n\n");
      return { content: [{ type: "text", text }] };
    },
  );

  server.tool(
    "fetch_page",
    "Fetch a documentation page as raw markdown. Use after search_docs to get the full content of a relevant page.",
    {
      slug: z
        .string()
        .describe(
          "Page slug, e.g. 'getting-started/what-is-arete' or 'sdks/typescript'. Omit leading slash and trailing .md.",
        ),
    },
    async ({ slug }) => {
      const cleanSlug = slug.replace(/^\/+|\/+$/g, "").replace(/\.md$/, "");
      const url = `${DOCS_BASE}/${cleanSlug}.md`;
      const res = await fetch(url);
      if (!res.ok) {
        return {
          content: [
            {
              type: "text",
              text: `Page not found: ${cleanSlug} (HTTP ${res.status}). Use search_docs to find valid slugs.`,
            },
          ],
          isError: true,
        };
      }
      const text = await res.text();
      return { content: [{ type: "text", text }] };
    },
  );

  return server;
}

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

// Vercel Node.js function handler. The MCP transport speaks JSON-RPC over a
// single HTTP endpoint (Streamable HTTP) — stateless mode, since each Vercel
// invocation is a fresh process.
export default async function handler(
  req: import("node:http").IncomingMessage,
  res: import("node:http").ServerResponse,
) {
  const server = buildServer();
  const transport = new StreamableHTTPServerTransport({
    sessionIdGenerator: undefined, // stateless: no session persistence between invocations
  });
  await server.connect(transport);
  await transport.handleRequest(req, res);
}
