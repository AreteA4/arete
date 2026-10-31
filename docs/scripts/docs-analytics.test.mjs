import assert from "node:assert/strict";
import test from "node:test";
import {
  classifyCopiedSnippet,
  codePreview,
  docsCtaHref,
  outboundDestination,
  sdkLanguageFromPath,
  sdkLanguageFromTabLabel,
  withUtm,
  xPostHref,
} from "../lib/docs-analytics.mjs";

test("x posts tag docs and home links with the X campaign convention", () => {
  assert.equal(
    xPostHref("https://docs.arete.run/using-stacks/quickstart/", {
      campaign: "2026-w44",
      slug: "quickstart",
      account: "usearete",
    }),
    "https://docs.arete.run/using-stacks/quickstart/?utm_source=x&utm_medium=social&utm_campaign=2026-w44&utm_content=quickstart_usearete",
  );
  assert.equal(
    xPostHref("https://arete.run/", {
      campaign: "2026-w44",
      slug: "home",
      account: "usearete",
    }),
    "https://arete.run/?utm_source=x&utm_medium=social&utm_campaign=2026-w44&utm_content=home_usearete",
  );
});

test("docs CTAs tag the marketing site without claiming the visit came from X", () => {
  assert.equal(
    docsCtaHref("https://arete.run/", "about_home"),
    "https://arete.run/?utm_source=docs&utm_medium=referral&utm_campaign=docs&utm_content=about_home",
  );
  assert.equal(
    docsCtaHref("https://arete.run/keys", "contact_keys"),
    "https://arete.run/keys?utm_source=docs&utm_medium=referral&utm_campaign=docs&utm_content=contact_keys",
  );
});

test("withUtm keeps a relative path and existing query", () => {
  assert.equal(
    withUtm("/using-stacks/quickstart/?ref=1", {
      source: "docs",
      medium: "referral",
      campaign: "docs",
      content: "home_quickstart",
    }),
    "/using-stacks/quickstart/?ref=1&utm_source=docs&utm_medium=referral&utm_campaign=docs&utm_content=home_quickstart",
  );
});

test("outbound destinations are github and x only", () => {
  assert.equal(
    outboundDestination("https://github.com/AreteA4/arete"),
    "github",
  );
  assert.equal(outboundDestination("https://x.com/usearete"), "x");
  assert.equal(outboundDestination("https://twitter.com/usearete"), "x");
  assert.equal(outboundDestination("https://arete.run/"), null);
  assert.equal(outboundDestination("/using-stacks/quickstart/"), null);
});

test("sdk language tabs and sdk paths normalize to a language", () => {
  assert.equal(
    sdkLanguageFromTabLabel("Pagination — TypeScript"),
    "typescript",
  );
  assert.equal(sdkLanguageFromTabLabel("Connect with React"), "react");
  assert.equal(sdkLanguageFromTabLabel("TypeScript (Non-React)"), "typescript");
  assert.equal(sdkLanguageFromTabLabel("macOS / Linux"), null);
  assert.equal(sdkLanguageFromPath("/sdks/rust/"), "rust");
  assert.equal(sdkLanguageFromPath("/sdks/react-sessions/"), "react");
  assert.equal(sdkLanguageFromPath("/sdks/validation/"), null);
  assert.equal(sdkLanguageFromPath("/using-stacks/quickstart/"), null);
});

test("copied install and CLI snippets are classified, other code is not", () => {
  assert.deepEqual(
    classifyCopiedSnippet("curl -fsSL https://arete.run/install.sh | sh"),
    {
      snippet_kind: "install_script",
      code_preview: "curl -fsSL https://arete.run/install.sh | sh",
    },
  );
  assert.equal(
    classifyCopiedSnippet("irm https://arete.run/install.ps1 | iex")
      ?.snippet_kind,
    "install_script",
  );
  assert.equal(
    classifyCopiedSnippet("a4 protocol init my-tip-jar\nnpm install")
      ?.snippet_kind,
    "cli",
  );
  assert.equal(
    classifyCopiedSnippet("npx @usearete/a4 install")?.snippet_kind,
    "cli",
  );
  assert.equal(
    classifyCopiedSnippet("npm install @usearete/sdk")?.snippet_kind,
    "package_install",
  );
  assert.equal(classifyCopiedSnippet("fn main() {}"), null);
  assert.equal(
    codePreview("npm install @usearete/sdk\nsecond"),
    "npm install @usearete/sdk",
  );
  assert.equal(codePreview("first\u007fsecond"), "first");
});
