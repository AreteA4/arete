import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  buildSearchIndex,
  documentTerms,
  queryTerms,
  searchDocs,
  sections,
  stripPageDirective,
} from "../lib/docs-search.mjs";

// Mirrors src/lib/markdownDirective(): the built .md twin of every page (and
// so the production docs-index.json) starts with this blockquote.
function directive(slug) {
  const path = slug ? `/${slug}.md` : "/index.md";
  return `> For the complete documentation index optimized for AI agents, see [llms.txt](https://docs.arete.run/llms.txt) or [llms-full.txt](https://docs.arete.run/llms-full.txt). A markdown version of this page is available at [${path}](https://docs.arete.run${path}) or by sending \`Accept: text/markdown\`.\n\n`;
}

const CONTENT = join(
  dirname(fileURLToPath(import.meta.url)),
  "..",
  "src",
  "content",
  "docs",
);

function* contentFiles(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) yield* contentFiles(path);
    else if (/\.mdx?$/.test(entry.name)) yield path;
  }
}

// Approximates the built .md endpoints the production index is made from:
// frontmatter title, the agent directive, body without MDX imports.
function sourcePages() {
  return [...contentFiles(CONTENT)].map((file) => {
    const text = readFileSync(file, "utf8");
    const match = /^---\n([\s\S]*?)\n---\n([\s\S]*)$/.exec(text);
    const front = match ? match[1] : "";
    const body = (match ? match[2] : text)
      .split("\n")
      .filter((line) => !/^import\s.+from\s/.test(line))
      .join("\n");
    const title = /^title:\s*(.+)$/m
      .exec(front)?.[1]
      ?.replace(/^["']|["']$/g, "");
    const slug = relative(CONTENT, file)
      .replace(/\.mdx?$/, "")
      .replace(/(^|\/)index$/, "");
    return {
      slug,
      title: title ?? slug,
      content: directive(slug) + body.trim(),
    };
  });
}

const index = buildSearchIndex(sourcePages());

function topSlugs(query, limit = 3) {
  return searchDocs(index, query, limit).map((result) => result.slug);
}

function assertInTop(query, expected, limit = 3) {
  const slugs = topSlugs(query, limit);
  const wanted = Array.isArray(expected) ? expected : [expected];
  assert.ok(
    wanted.some((slug) => slugs.includes(slug)),
    `"${query}" → [${slugs.join(", ")}]; expected one of [${wanted.join(", ")}] in the top ${limit}`,
  );
}

test("query terms drop stopwords, lower-case and singularise", () => {
  assert.deepEqual(
    queryTerms("How do I use publishable keys with the browser?"),
    ["publishable", "key", "browser"],
  );
  assert.deepEqual(queryTerms("AreteProvider React"), [
    "areteprovider",
    "react",
  ]);
  assert.deepEqual(queryTerms("the and of"), []);
});

test("document terms index identifiers whole and by their parts", () => {
  const terms = documentTerms(
    "wrap the app in AreteProvider and call deployWithCheckpoint",
  );
  for (const term of [
    "areteprovider",
    "arete",
    "provider",
    "deploywithcheckpoint",
    "deploy",
    "checkpoint",
  ]) {
    assert.ok(terms.includes(term), `missing ${term}`);
  }
});

test("sections split on headings but not inside fenced code", () => {
  const parsed = sections(
    "intro\n## Setup\ntext\n```bash\n# not a heading\n```\n## Next\nmore",
  );
  assert.deepEqual(
    parsed.map((section) => section.heading),
    ["", "Setup", "Next"],
  );
  assert.match(parsed[1].text, /# not a heading/);
});

test("multi-word queries match on their terms, not as one exact phrase", () => {
  const small = buildSearchIndex([
    {
      slug: "sdks/react",
      title: "React",
      content: "## Provider\nWrap your app in AreteProvider.",
    },
    { slug: "other", title: "Other", content: "Nothing relevant here." },
  ]);
  assert.equal(searchDocs(small, "AreteProvider React")[0].slug, "sdks/react");
  assert.equal(searchDocs(small, "arete provider")[0].slug, "sdks/react");
  assert.equal(searchDocs(small, "provider")[0].section, "Provider");
  assert.deepEqual(searchDocs(small, "zzz"), []);
});

// Pages that explain browser publishable keys; there is no single auth page.
const BROWSER_KEY_PAGES = [
  "developers",
  "sdks/react",
  "using-stacks/connect",
  "using-stacks/transactions",
  "building-stacks/your-first-stack",
];

// Most of these returned nothing when the whole query had to appear verbatim.
test("known task queries find their pages in the top three", () => {
  assertInTop("AreteProvider React", "sdks/react");
  assertInTop("wallet adapter TypeScript SDK", [
    "sdks/typescript",
    "using-stacks/transactions",
  ]);
  assertInTop("wallet adapter transactions", "using-stacks/transactions");
  assertInTop("using stacks transactions", "using-stacks/transactions");
  assertInTop("publishable browser key", BROWSER_KEY_PAGES);
  assertInTop("publishable keys auth browser", BROWSER_KEY_PAGES);
  assertInTop("deployWithCheckpoint mutation", "sdks/react");
  assertInTop("compose stack", "cli/commands");
  assertInTop("install a stack", "using-stacks/installation");
  assertInTop("doctor", "cli/commands");
});

test("a query naming a page's title ranks that page first", () => {
  assert.equal(topSlugs("AreteProvider React", 1)[0], "sdks/react");
  assert.equal(
    topSlugs("using stacks transactions", 1)[0],
    "using-stacks/transactions",
  );
});

test("the per-page agent directive is not searchable content", () => {
  const page = `${directive("sdks/react")}# React\n\nWrap the app.`;
  assert.equal(stripPageDirective(page), "# React\n\nWrap the app.");
  assert.equal(stripPageDirective("# Plain\n\ntext"), "# Plain\n\ntext");
  // Words in the directive must not match every page through it.
  const results = searchDocs(index, "documentation index", 60);
  assert.ok(
    results.length < index.documents.length / 2,
    `"documentation index" matched ${results.length} of ${index.documents.length} pages`,
  );
  for (const result of searchDocs(index, "React", 5)) {
    assert.doesNotMatch(result.snippet, /complete documentation index/);
  }
});

test("stopwords are dropped before stemming", () => {
  assert.deepEqual(queryTerms("does it work"), ["work"]);
});
