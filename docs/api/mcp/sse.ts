// MCP server for Arete documentation, deployed as a Vercel serverless function.
// Vercel auto-detects api/*.ts files and runs them as Node functions independently
// of the static Astro build, so we don't need to switch Astro to hybrid/server mode.
//
// Endpoint: https://docs.arete.run/mcp (canonical), also reachable at
// /mcp/sse for older links via rewrites in vercel.json.
//
// Tools exposed:
//   - search_docs(query): keyword search across docs-index.json (per-page chunks)
//   - fetch_page(slug):   returns raw markdown for a single doc page
//
// Discovery manifest lives at /.well-known/mcp.json.
// docs-index.json is generated at build time by scripts/build-docs-index.mjs
// from the per-page .md endpoints, so each entry has { slug, title, content }.

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/streamableHttp.js";
import { z } from "zod";

const DOCS_BASE = "https://docs.arete.run";
const MCP_ENDPOINT = `${DOCS_BASE}/mcp`;
const FETCH_TIMEOUT_MS = 8000;
const INDEX_TTL_MS = 5 * 60 * 1000;

const TOOL_DEFINITIONS = {
  search_docs: {
    name: "search_docs",
    description:
      "Search the Arete documentation. Returns matching page snippets ranked by relevance. Use this when answering questions about Arete features, the Rust DSL, SDKs, or CLI.",
    inputSchema: {
      type: "object",
      properties: {
        query: {
          type: "string",
          description: "Search query — keywords or a full question",
        },
        limit: {
          type: "number",
          minimum: 1,
          maximum: 20,
          default: 5,
          description: "Max number of results to return",
        },
      },
      required: ["query"],
    },
    operationId: "search_docs",
  },
  fetch_page: {
    name: "fetch_page",
    description:
      "Fetch a documentation page as raw markdown. Use after search_docs to get the full content of a relevant page.",
    inputSchema: {
      type: "object",
      properties: {
        slug: {
          type: "string",
          description:
            "Page slug, e.g. 'getting-started/what-is-arete' or 'sdks/typescript'. Omit leading slash and trailing .md.",
        },
      },
      required: ["slug"],
    },
    operationId: "fetch_page",
  },
};

const RESOURCE_DEFINITIONS = [
  {
    uri: "https://docs.arete.run/skill.md",
    name: "arete-platform",
    description:
      "Official Arete agent skill for onboarding, API keys, registry access, CLI setup, SDK usage, and stack-building workflows.",
    mimeType: "text/markdown",
  },
];

interface DocPage {
  slug: string;
  title: string;
  content: string;
}

// Cached index across warm invocations on the same Vercel function instance.
// Stateless across cold starts (the JSON is CDN-cacheable anyway), but saves
// the extra hop on warm reuse.
let indexCache: { pages: DocPage[]; fetchedAt: number } | null = null;

async function getIndex(): Promise<DocPage[]> {
  const now = Date.now();
  if (indexCache && now - indexCache.fetchedAt < INDEX_TTL_MS) {
    return indexCache.pages;
  }
  const res = await fetch(`${DOCS_BASE}/docs-index.json`, {
    signal: AbortSignal.timeout(FETCH_TIMEOUT_MS),
  });
  if (!res.ok) {
    throw new Error(`Failed to load docs-index.json: HTTP ${res.status}`);
  }
  const pages = (await res.json()) as DocPage[];
  indexCache = { pages, fetchedAt: now };
  return pages;
}

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
      let pages: DocPage[];
      try {
        pages = await getIndex();
      } catch (err) {
        return {
          content: [
            {
              type: "text",
              text: `Failed to load docs index: ${(err as Error).message}`,
            },
          ],
          isError: true,
        };
      }
      const q = query.toLowerCase();
      const qRe = new RegExp(escapeRegex(q), "g");
      const scored = pages
        .map((page) => {
          const titleHits = (page.title.toLowerCase().match(qRe) ?? []).length;
          const contentHits = (page.content.toLowerCase().match(qRe) ?? [])
            .length;
          // Title hits weighted more heavily than body hits.
          const score = titleHits * 5 + contentHits;
          return {
            slug: page.slug,
            title: page.title,
            snippet: extractSnippet(page.content, q),
            score,
          };
        })
        .filter((p) => p.score > 0)
        .sort((a, b) => b.score - a.score)
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
            `## Result ${i + 1}: ${p.title}\n` +
            `slug: \`${p.slug}\`  (call \`fetch_page\` with this slug for full content)\n\n` +
            `${p.snippet}`,
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
      // Empty/root slug maps to /index.md; the docs index stores the homepage
      // with slug "" but it's only reachable at /index.md, not /.md.
      const cleanSlug =
        slug.replace(/^\/+|\/+$/g, "").replace(/\.md$/, "") || "index";
      const url = `${DOCS_BASE}/${cleanSlug}.md`;
      try {
        const res = await fetch(url, {
          signal: AbortSignal.timeout(FETCH_TIMEOUT_MS),
        });
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
      } catch (err) {
        return {
          content: [
            {
              type: "text",
              text: `Failed to fetch ${cleanSlug}: ${(err as Error).message}`,
            },
          ],
          isError: true,
        };
      }
    },
  );

  return server;
}

function buildDescriptor() {
  return {
    server: {
      name: "Arete Documentation",
      version: "1.0.0",
      transport: "http",
      endpoint: MCP_ENDPOINT,
    },
    capabilities: {
      tools: TOOL_DEFINITIONS,
      resources: RESOURCE_DEFINITIONS,
      prompts: [],
    },
  };
}

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

// Returns ~600 chars of context around the first occurrence of `query` in
// `content`, falling back to the document head if there's no hit (which
// happens when the score came purely from a title match).
function extractSnippet(content: string, query: string): string {
  const idx = content.toLowerCase().indexOf(query);
  if (idx < 0) return content.slice(0, 600).trim();
  const start = Math.max(0, idx - 150);
  const end = Math.min(content.length, idx + 450);
  const prefix = start > 0 ? "…" : "";
  const suffix = end < content.length ? "…" : "";
  return prefix + content.slice(start, end).trim() + suffix;
}

function acceptsEventStream(
  acceptHeader: string | string[] | undefined,
): boolean {
  const accept = Array.isArray(acceptHeader)
    ? acceptHeader.join(",")
    : (acceptHeader ?? "");
  return accept.toLowerCase().includes("text/event-stream");
}

function sendJson(
  res: import("node:http").ServerResponse,
  body: unknown,
  statusCode = 200,
) {
  res.statusCode = statusCode;
  res.setHeader("Content-Type", "application/json; charset=utf-8");
  res.setHeader("Cache-Control", "public, max-age=3600");
  res.setHeader("Access-Control-Allow-Origin", "*");
  res.setHeader("Access-Control-Allow-Methods", "GET, POST, OPTIONS");
  res.setHeader(
    "Access-Control-Allow-Headers",
    "Content-Type, Accept, MCP-Protocol-Version",
  );
  res.setHeader("X-Content-Type-Options", "nosniff");
  res.end(JSON.stringify(body, null, 2));
}

function sendCorsNoContent(res: import("node:http").ServerResponse) {
  res.statusCode = 204;
  res.setHeader("Access-Control-Max-Age", "3600");
  res.setHeader("Access-Control-Allow-Origin", "*");
  res.setHeader("Access-Control-Allow-Methods", "GET, POST, OPTIONS");
  res.setHeader(
    "Access-Control-Allow-Headers",
    "Content-Type, Accept, MCP-Protocol-Version",
  );
  res.setHeader("X-Content-Type-Options", "nosniff");
  res.end();
}

// Vercel Node.js function handler. The MCP transport speaks JSON-RPC over a
// single HTTP endpoint (Streamable HTTP) — stateless mode, since each Vercel
// invocation is a fresh process.
export default async function handler(
  req: import("node:http").IncomingMessage,
  res: import("node:http").ServerResponse,
) {
  if (req.method === "OPTIONS") {
    sendCorsNoContent(res);
    return;
  }

  if (req.method === "GET" && !acceptsEventStream(req.headers.accept)) {
    sendJson(res, buildDescriptor());
    return;
  }

  const server = buildServer();
  const transport = new StreamableHTTPServerTransport({
    sessionIdGenerator: undefined, // stateless: no session persistence between invocations
  });
  res.setHeader("Access-Control-Allow-Origin", "*");
  res.setHeader("Access-Control-Allow-Methods", "GET, POST, OPTIONS");
  res.setHeader(
    "Access-Control-Allow-Headers",
    "Content-Type, Accept, MCP-Protocol-Version",
  );
  await server.connect(transport);
  await transport.handleRequest(req, res);
}
