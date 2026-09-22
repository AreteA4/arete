import assert from "node:assert/strict";
import test from "node:test";
import {
  json404,
  json404Body,
  jsonErrorBody,
  markdown404,
  markdown404Body,
  markdownTwinPath,
  prefersJson,
  prefersMarkdown,
} from "../lib/agent-http.mjs";

test("prefersMarkdown matches Accept: text/markdown", () => {
  assert.equal(prefersMarkdown("text/markdown"), true);
  assert.equal(prefersMarkdown("text/markdown, text/html;q=0.9"), true);
  assert.equal(prefersMarkdown("text/html"), false);
});

test("prefersMarkdown rejects an explicit q=0", () => {
  assert.equal(prefersMarkdown("text/markdown;q=0, text/html;q=1"), false);
  assert.equal(prefersMarkdown("text/markdown;q=0.0"), false);
  assert.equal(prefersMarkdown("text/html,application/xhtml+xml,*/*;q=0.8"), false);
});

test("prefersJson ignores browser HTML Accept lists and q=0", () => {
  assert.equal(prefersJson("application/json"), true);
  assert.equal(prefersJson("application/json, text/html"), false);
  assert.equal(prefersJson("text/html,application/xhtml+xml"), false);
  assert.equal(prefersJson("application/json;q=0"), false);
});

test("markdown twin of homepage is /index.md", () => {
  assert.equal(markdownTwinPath("/"), "/index.md");
  assert.equal(
    markdownTwinPath("/using-stacks/quickstart/"),
    "/using-stacks/quickstart.md",
  );
});

test("markdown 404 is HTTP 404 with a long body and llms.txt link", () => {
  const body = markdown404Body("/__ora-404-probe");
  assert.ok(body.length >= 20);
  assert.match(body, /llms\.txt/);
  assert.match(body, /docs\.arete\.run/);
  const res = markdown404("/__ora-404-probe");
  assert.equal(res.status, 404);
  assert.match(res.headers["content-type"], /text\/markdown/);
  assert.equal(res.headers.vary, "Accept");
});

test("JSON errors include code, message, and hint", () => {
  const payload = jsonErrorBody("not_found", "missing", "see llms.txt");
  assert.equal(payload.error.code, "not_found");
  assert.equal(payload.error.message, "missing");
  assert.equal(payload.error.hint, "see llms.txt");
  const body = json404Body("/nope");
  assert.equal(body.error.code, "not_found");
  const res = json404("/nope");
  assert.equal(res.status, 404);
  assert.match(res.headers["content-type"], /application\/json/);
  const parsed = JSON.parse(res.body);
  assert.equal(parsed.error.code, "not_found");
});
