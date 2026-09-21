#!/usr/bin/env node
// Local static server that mimics the Vercel Edge Accept rewrite.
// Used only for AFDocs / is-agentic scoring of the built dist/. Not deployed.

import { createServer } from "node:http";
import { existsSync, statSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { extname, join, normalize } from "node:path";
import {
  isDiscoveryPath,
  json404,
  markdown404,
  markdownHeaders,
  markdownTwinPath,
  prefersJson,
  prefersMarkdown,
} from "../lib/agent-http.mjs";

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
  ".png": "image/png",
  ".woff2": "font/woff2",
  ".ico": "image/x-icon",
};

function isFile(path) {
  try {
    return statSync(path).isFile();
  } catch {
    return false;
  }
}

function resolve(pathname) {
  if (pathname === "/.well-known/agent-skills") {
    pathname = "/.well-known/agent-skills/index.json";
  }
  if (pathname === "/.well-known/mcp") {
    pathname = "/.well-known/mcp.json";
  }
  const rel = normalize(pathname).replace(/^(\.\.[/\\])+/, "");
  const candidate = join(DIST, rel);
  if (isFile(candidate)) {
    return candidate;
  }
  if (isFile(join(candidate, "index.html"))) {
    return join(candidate, "index.html");
  }
  if (isFile(candidate + ".html")) {
    return candidate + ".html";
  }
  return null;
}

function send(res, status, headers, body) {
  res.writeHead(status, headers);
  res.end(body);
}

createServer(async (req, res) => {
  try {
    const url = new URL(req.url || "/", `http://127.0.0.1:${PORT}`);
    let pathname = url.pathname;
    const accept = req.headers.accept || "";
    const markdown = prefersMarkdown(accept);
    const json = prefersJson(accept);

    if (
      markdown &&
      !isDiscoveryPath(pathname) &&
      !pathname.endsWith(".md") &&
      !pathname.endsWith(".txt")
    ) {
      const twin = markdownTwinPath(pathname);
      const file = resolve(twin);
      if (file) {
        const body = await readFile(file);
        send(res, 200, markdownHeaders(), body);
        return;
      }
      const miss = markdown404(pathname);
      send(res, miss.status, miss.headers, miss.body);
      return;
    }

    const file = resolve(pathname);
    if (!file) {
      if (json) {
        const miss = json404(pathname);
        send(res, miss.status, miss.headers, miss.body);
        return;
      }
      if (markdown) {
        const miss = markdown404(pathname);
        send(res, miss.status, miss.headers, miss.body);
        return;
      }
      const html404 = join(DIST, "404.html");
      if (existsSync(html404)) {
        send(
          res,
          404,
          { "content-type": "text/html; charset=utf-8", vary: "Accept" },
          await readFile(html404),
        );
        return;
      }
      send(res, 404, { "content-type": "text/plain; charset=utf-8" }, "Not Found");
      return;
    }

    const body = await readFile(file);
    const type = TYPES[extname(file)] || "application/octet-stream";
    const headers = {
      "content-type": type,
      "cache-control": "public, max-age=0, must-revalidate",
      vary: "Accept",
      "x-llms-txt": "https://docs.arete.run/llms.txt",
      link: '</llms.txt>; rel="llms-txt", </llms-full.txt>; rel="llms-full-txt", </.well-known/mcp/server-card.json>; rel="mcp-server-card", </.well-known/agent-card.json>; rel="agent-card", </.well-known/agent-skills>; rel="agent-skills", </openapi.json>; rel="describedby"',
    };
    if (extname(file) === ".md") {
      Object.assign(headers, markdownHeaders());
    }
    send(res, 200, headers, body);
  } catch (err) {
    send(res, 500, { "content-type": "text/plain" }, String(err));
  }
}).listen(PORT, "127.0.0.1", () => {
  console.log(`score-server listening on http://127.0.0.1:${PORT}`);
});
