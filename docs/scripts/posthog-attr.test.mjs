import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const initPath = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "../public/posthog-init.js",
);
const source = fs.readFileSync(initPath, "utf8");
const start = source.indexOf("/* arete-attr:start */");
const end = source.indexOf("/* arete-attr:end */");
const block = source.slice(start, end);

const { readAreteAttr, applyAreteAttr } = new Function(
  `${block}
  return { readAreteAttr, applyAreteAttr };`,
)();

const COOKIE =
  "theme=dark; arete_attr=utm_source=x&utm_medium=social&utm_campaign=2026-w44&utm_content=quickstart_usearete";

test("the marked block is wired into PostHog before_send", () => {
  assert.ok(start !== -1 && end > start);
  assert.match(source, /before_send:\s*applyAreteAttr/);
});

test("a valid arete_attr cookie parses the attribution", () => {
  assert.deepEqual(readAreteAttr(COOKIE), {
    utm_source: "x",
    utm_medium: "social",
    utm_campaign: "2026-w44",
    utm_content: "quickstart_usearete",
  });
});

test("a plus in utm_content survives cookie decoding", () => {
  const literal =
    "arete_attr=utm_source=x&utm_medium=social&utm_campaign=2026-w44&utm_content=savings+demo";
  const encoded =
    "arete_attr=" +
    encodeURIComponent(
      "utm_source=x&utm_medium=social&utm_campaign=2026-w44&utm_content=savings+demo",
    );
  for (const header of [literal, encoded]) {
    assert.equal(readAreteAttr(header).utm_content, "savings+demo");
  }
});

test("forged cookies are rejected", () => {
  assert.equal(
    readAreteAttr(
      "arete_attr=utm_source=evil&utm_medium=social&utm_campaign=2026-w44",
    ),
    null,
  );
  assert.equal(
    readAreteAttr(
      "arete_attr=utm_source=x&utm_medium=email&utm_campaign=2026-w44",
    ),
    null,
  );
  assert.equal(
    readAreteAttr(
      "arete_attr=utm_source=x&utm_medium=social&utm_campaign=<script>",
    ),
    null,
  );
  assert.equal(
    readAreteAttr("arete_attr=utm_source=x&utm_medium=social&injected=1"),
    null,
  );
  assert.equal(readAreteAttr(""), null);
  assert.equal(readAreteAttr("theme=dark"), null);
});

test("UTMs on the current URL win over the cookie", () => {
  const event = applyAreteAttr(
    {
      event: "$autocapture",
      properties: { utm_source: "registered", utm_content: "" },
    },
    COOKIE,
    "?utm_source=docs",
  );
  assert.equal(event.properties.utm_source, "docs");
  assert.equal(event.properties.utm_medium, "social");
  assert.equal(event.properties.utm_campaign, "2026-w44");
  assert.equal(event.properties.utm_content, "quickstart_usearete");
  assert.equal(event.properties.$set, undefined);
});

test("pageviews set initial UTMs once and current UTMs", () => {
  const event = applyAreteAttr(
    {
      event: "$pageview",
      properties: { utm_campaign: "registered-A" },
    },
    COOKIE,
    "?utm_campaign=from-query",
  );
  assert.equal(event.properties.utm_campaign, "from-query");
  assert.equal(event.properties.$set.utm_source, "x");
  assert.equal(event.properties.$set.utm_campaign, "from-query");
  assert.equal(event.properties.$set_once.$initial_utm_source, "x");
  assert.equal(event.properties.$set_once.$initial_utm_medium, "social");
  assert.equal(event.properties.$set_once.$initial_utm_campaign, "from-query");
  assert.equal(
    event.properties.$set_once.$initial_utm_content,
    "quickstart_usearete",
  );
});

test("a fresh cookie replaces registered UTMs when the URL has none", () => {
  const cookieB =
    "arete_attr=utm_source=x&utm_medium=social&utm_campaign=2026-w45&utm_content=savings_usearete";
  const event = applyAreteAttr(
    {
      event: "$pageview",
      properties: {
        utm_source: "x",
        utm_medium: "social",
        utm_campaign: "2026-w44",
        utm_content: "quickstart_usearete",
        $set: {
          utm_source: "x",
          utm_medium: "social",
          utm_campaign: "2026-w44",
          utm_content: "quickstart_usearete",
        },
        $set_once: {
          $initial_utm_source: "x",
          $initial_utm_medium: "social",
          $initial_utm_campaign: "2026-w44",
          $initial_utm_content: "quickstart_usearete",
        },
      },
    },
    cookieB,
    "",
  );
  assert.equal(event.properties.utm_source, "x");
  assert.equal(event.properties.utm_medium, "social");
  assert.equal(event.properties.utm_campaign, "2026-w45");
  assert.equal(event.properties.utm_content, "savings_usearete");
  assert.equal(event.properties.$set.utm_campaign, "2026-w45");
  assert.equal(event.properties.$set.utm_content, "savings_usearete");
  assert.equal(event.properties.$set_once.$initial_utm_campaign, "2026-w45");
  assert.equal(
    event.properties.$set_once.$initial_utm_content,
    "savings_usearete",
  );
});

test("snapshot events are left alone", () => {
  const event = {
    event: "$snapshot",
    properties: { $snapshot_data: [] },
  };
  assert.equal(applyAreteAttr(event, COOKIE), event);
  assert.deepEqual(event.properties, { $snapshot_data: [] });
});

test("a missing event is ignored", () => {
  assert.equal(applyAreteAttr(null, COOKIE), null);
});
