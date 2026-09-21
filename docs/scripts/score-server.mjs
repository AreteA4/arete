#!/usr/bin/env node
// Local static server that mimics the Vercel Edge Accept: text/markdown rewrite.
// Used only for AFDocs scoring of the built dist/. Not deployed.

import { createServer } from "node:http";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { extname, join, normalize } from "node:path";

const DIST = "dist";
const PORT = Number(process.env.PORT || 4322);
const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".md": "text/markdown; charset=utf-8",
  ".txt": "text/plain; charset=utf-8",
  ".xml": "application/xml; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".svg": "image/svg+xml",
  ".woff2": "font/woff2",
  ".ico": "image/x-icon",
};

function resolve(pathname) {
  if (pathname === "/.well-known/agent-skills") {
    pathname = "/.well-known/agent-skills/index.json";
  }
  if (pathname === "/.well-known/mcp") {
    pathname = "/.well-known/mcp.json";
  }
  const rel = normalize(pathname).replace(/^(\.\.[/\\])+/, "");
  const candidate = join(DIST, rel);
  if (existsSync(candidate) && !candidate.endsWith("/")) {
    return candidate;
  }
  if (existsSync(join(candidate, "index.html"))) {
    return join(candidate, "index.html");
  }
  if (existsSync(candidate + ".html")) {
    return candidate + ".html";
  }
  return null;
}

createServer(async (req, res) => {
  try {
    const url = new URL(req.url || "/", `http://127.0.0.1:${PORT}`);
    let pathname = url.pathname;
    const accept = req.headers.accept || "";
    const alreadyMd = pathname.endsWith(".md") || pathname.endsWith(".txt");
    if (
      /\btext\/markdown\b/i.test(accept) &&
      !alreadyMd &&
      !pathname.includes(".")
    ) {
      pathname =
        pathname === "/" ? "/index.md" : `${pathname.replace(/\/$/, "")}.md`;
    }
    const file = resolve(pathname);
    if (!file) {
      res.writeHead(404, { "content-type": "text/plain" });
      res.end("Not Found");
      return;
    }
    const body = await readFile(file);
    const type = TYPES[extname(file)] || "application/octet-stream";
    const headers = {
      "content-type": type,
      "cache-control": "public, max-age=0, must-revalidate",
      "x-llms-txt": "https://docs.arete.run/llms.txt",
      link: '</llms.txt>; rel="llms-txt", </llms-full.txt>; rel="llms-full-txt", </.well-known/mcp/server-card.json>; rel="mcp-server-card", </.well-known/agent-card.json>; rel="agent-card", </.well-known/agent-skills>; rel="agent-skills"',
    };
    res.writeHead(200, headers);
    res.end(body);
  } catch (err) {
    res.writeHead(500, { "content-type": "text/plain" });
    res.end(String(err));
  }
}).listen(PORT, "127.0.0.1", () => {
  console.log(`score-server listening on http://127.0.0.1:${PORT}`);
});
