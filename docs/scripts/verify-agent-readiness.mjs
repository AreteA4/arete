#!/usr/bin/env node
// Verify is-agentic / AFDocs local checks against dist/ via score-server.
// Usage: npm run build && npm run verify:agent

import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";

const PORT = Number(process.env.PORT || 4322);
const BASE = `http://127.0.0.1:${PORT}`;
const DIST = "dist";

const failures = [];

function fail(name, detail) {
  failures.push(`${name}: ${detail}`);
  console.error(`FAIL  ${name}: ${detail}`);
}

function ok(name, detail = "") {
  console.log(`PASS  ${name}${detail ? ` (${detail})` : ""}`);
}

async function req(path, headers = {}, method = "GET") {
  const res = await fetch(BASE + path, { method, headers, redirect: "manual" });
  const buf = Buffer.from(await res.arrayBuffer());
  return {
    status: res.status,
    headers: res.headers,
    body: buf.toString("utf8"),
    buf,
  };
}

function header(res, name) {
  return res.headers.get(name) || "";
}

async function main() {
  if (!existsSync(`${DIST}/index.html`) || !existsSync(`${DIST}/llms.txt`)) {
    throw new Error("dist/ is missing; run npm run build first");
  }

  const child = spawn(process.execPath, ["scripts/score-server.mjs"], {
    stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env, PORT: String(PORT) },
  });
  const stderrChunks = [];
  child.stderr.on("data", (d) => {
    stderrChunks.push(d);
  });

  try {
    await new Promise((resolve, reject) => {
      const t = setTimeout(() => {
        const err = Buffer.concat(stderrChunks).toString("utf8").trim();
        reject(
          new Error(
            err
              ? `score-server start timeout: ${err}`
              : "score-server start timeout",
          ),
        );
      }, 8000);
      const done = (fn) => (value) => {
        clearTimeout(t);
        fn(value);
      };
      child.stdout.on("data", (d) => {
        if (String(d).includes("listening")) done(resolve)();
      });
      child.on("error", done(reject));
      child.once("exit", (code, signal) => {
        const err = Buffer.concat(stderrChunks).toString("utf8").trim();
        done(reject)(
          new Error(
            `score-server exited before listening (code=${code}, signal=${signal})${err ? `: ${err}` : ""}`,
          ),
        );
      });
    });

    const mdHome = await req("/", { accept: "text/markdown" });
    if (mdHome.status !== 200) fail("markdown homepage status", mdHome.status);
    else if (!/text\/markdown/i.test(header(mdHome, "content-type"))) {
      fail("markdown homepage content-type", header(mdHome, "content-type"));
    } else if (!/accept/i.test(header(mdHome, "vary"))) {
      fail("markdown homepage Vary", header(mdHome, "vary") || "none");
    } else if (mdHome.body.trim().length < 20) {
      fail("markdown homepage body", "too short");
    } else ok("markdown homepage", `${mdHome.body.length} bytes`);

    const htmlHome = await req("/", { accept: "text/html" });
    if (htmlHome.status !== 200) fail("html homepage status", htmlHome.status);
    else if (!/text\/html/i.test(header(htmlHome, "content-type"))) {
      fail("html homepage content-type", header(htmlHome, "content-type"));
    } else ok("html homepage");

    const probe = "/__ora-404-probe-local";
    const md404 = await req(probe, { accept: "text/markdown" });
    if (md404.status !== 404) fail("markdown 404 status", md404.status);
    else if (!/text\/markdown/i.test(header(md404, "content-type"))) {
      fail("markdown 404 content-type", header(md404, "content-type"));
    } else if (md404.body.trim().length < 20 || !/llms\.txt/.test(md404.body)) {
      fail("markdown 404 body", md404.body.slice(0, 120));
    } else ok("markdown 404");

    const json404 = await req(probe, { accept: "application/json" });
    if (json404.status !== 404) fail("json 404 status", json404.status);
    else {
      let parsed;
      try {
        parsed = JSON.parse(json404.body);
      } catch (e) {
        fail("json 404 parse", e.message);
      }
      if (parsed && parsed.error?.code && parsed.error?.message && parsed.error?.hint) {
        ok("json 404");
      } else if (parsed) {
        fail("json 404 shape", json404.body.slice(0, 200));
      }
    }

    const spec = await req("/openapi.json");
    if (spec.status !== 200) fail("openapi.json status", spec.status);
    else {
      const parsed = JSON.parse(spec.body);
      const ops = Object.values(parsed.paths || {}).flatMap((p) =>
        Object.values(p).map((op) => op.operationId).filter(Boolean),
      );
      const missing = ["getAreteDocsMcpDescriptor", "postAreteDocsMcpJsonRpc", "getAreteDocsOpenApiSpec"].filter(
        (id) => !ops.includes(id),
      );
      if (parsed.openapi !== "3.1.0") fail("openapi version", parsed.openapi);
      else if (missing.length) fail("openapi operationIds", missing.join(","));
      else ok("openapi.json", `${ops.length} operations`);
    }

    const llms = await req("/llms.txt");
    if (!/when to use arete/i.test(llms.body)) fail("llms when-to-use", "section missing");
    else ok("llms when-to-use");

    for (const path of ["/developers", "/about", "/contact", "/privacy"]) {
      const page = await req(path, { accept: "text/html" });
      const md = await req(path, { accept: "text/markdown" });
      if (page.status !== 200) fail(`${path} html`, page.status);
      else if (page.body.replace(/<[^>]+>/g, "").length < 500) {
        fail(`${path} content`, "under 500 characters");
      } else if (md.status !== 200 || !/text\/markdown/i.test(header(md, "content-type"))) {
        fail(`${path} markdown`, `${md.status} ${header(md, "content-type")}`);
      } else ok(path);
    }

    const homeHtml = await req("/");
    if (!/application\/ld\+json/.test(homeHtml.body)) fail("json-ld", "missing");
    else ok("json-ld");
    if (!/property="og:image"/.test(homeHtml.body) && !/property='og:image'/.test(homeHtml.body)) {
      fail("og:image", "missing");
    } else ok("og:image");

    const og = await req("/og.png");
    if (og.status !== 200) fail("og.png", og.status);
    else ok("og.png", `${og.buf.length} bytes`);

    const skills = await req("/.well-known/agent-skills", {
      accept: "text/markdown",
    });
    if (skills.status !== 200) fail("well-known skills markdown accept", skills.status);
    else if (!/json/i.test(header(skills, "content-type"))) {
      fail("well-known skills stayed JSON", header(skills, "content-type"));
    } else ok("well-known skills ignore markdown Accept");
  } finally {
    child.kill("SIGTERM");
  }

  if (failures.length) {
    console.error(`\n${failures.length} failed`);
    process.exit(1);
  }
  console.log("\nall agent-readiness checks passed");
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
