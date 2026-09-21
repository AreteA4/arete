// Postbuild: rewrite llms.txt into a Mintlify-style page map (title +
// description + .md link per page), copy sitemap-0.xml to /sitemap.xml,
// and mirror llms.txt under /.well-known/llms.txt.
//
// starlight-llms-txt still generates llms-full.txt / llms-small.txt.
// This script only replaces the sparse dump-pointer llms.txt.

import {
  copyFile,
  mkdir,
  readdir,
  readFile,
  writeFile,
} from "node:fs/promises";
import { join, relative } from "node:path";

const DIST = "dist";
const SITE = "https://docs.arete.run";
const SKIP_NAMES = new Set([
  "llms.txt",
  "llms-full.txt",
  "llms-small.txt",
  "skill.md",
  "SKILL.md",
  "agent.md",
]);

const SECTION_TITLES = {
  "": "Docs",
  "getting-started": "Getting started",
  "using-stacks": "Using stacks",
  "using-programs": "Using programs",
  concepts: "Concepts",
  "agent-skills": "Agents",
  "building-stacks": "Create live data",
  sdks: "SDK reference",
  cli: "CLI",
  "a4-server": "Self-hosting",
};

const PROMOTE = [
  "",
  "getting-started/what-is-arete",
  "using-stacks/quickstart",
  "agent-skills/explore-on-chain",
  "getting-started/from-question-to-app",
  "concepts/programs-views-stacks",
  "using-programs/overview",
  "agent-skills/overview",
  "agent-skills/mcp",
  "sdks/typescript",
  "sdks/react",
];

async function* walkMarkdown(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  for (const e of entries) {
    const p = join(dir, e.name);
    if (e.isDirectory()) {
      if (e.name === ".well-known" || e.name === "_astro") continue;
      yield* walkMarkdown(p);
    } else if (e.isFile() && p.endsWith(".md") && !SKIP_NAMES.has(e.name)) {
      yield p;
    }
  }
}

function parseFrontmatter(text) {
  const match = /^---\n([\s\S]*?)\n---\n([\s\S]*)$/.exec(text);
  if (!match) return { data: {}, body: text };
  const data = {};
  for (const line of match[1].split("\n")) {
    const m = /^([A-Za-z_][A-Za-z0-9_]*):\s*(.+)$/.exec(line);
    if (!m) continue;
    try {
      data[m[1]] = JSON.parse(m[2]);
    } catch {
      data[m[1]] = m[2];
    }
  }
  return { data, body: match[2].trim() };
}

function sectionOf(slug) {
  if (!slug) return "";
  const slash = slug.indexOf("/");
  return slash === -1 ? slug : slug.slice(0, slash);
}

const pages = [];
for await (const file of walkMarkdown(DIST)) {
  const text = await readFile(file, "utf8");
  const { data } = parseFrontmatter(text);
  const rel = relative(DIST, file).replace(/\.md$/, "");
  const slug = rel === "index" ? "" : rel;
  const title = typeof data.title === "string" ? data.title : slug || "Arete";
  const description =
    typeof data.description === "string"
      ? data.description.replace(/\s+/g, " ").trim()
      : "";
  pages.push({ slug, title, description });
}

pages.sort((a, b) => a.slug.localeCompare(b.slug));

const bySlug = new Map(pages.map((p) => [p.slug, p]));
const promoted = [];
const seen = new Set();
for (const slug of PROMOTE) {
  const page = bySlug.get(slug);
  if (!page) continue;
  promoted.push(page);
  seen.add(slug);
}

const rest = pages.filter((p) => !seen.has(p.slug));
const grouped = new Map();
for (const page of rest) {
  const key = sectionOf(page.slug);
  if (!grouped.has(key)) grouped.set(key, []);
  grouped.get(key).push(page);
}

function bullet(page) {
  const href = `${SITE}${page.slug ? `/${page.slug}` : "/index"}.md`;
  return page.description
    ? `- [${page.title}](${href}): ${page.description}`
    : `- [${page.title}](${href})`;
}

const lines = [
  "# Arete",
  "",
  "> Arete is an agent-first Solana application toolkit. Discover programs and live views, explore on-chain state through MCP, and install typed SDKs for reads, transactions, flows, and real-time data. This file indexes the markdown version of every page on docs.arete.run. Append `.md` to any page URL (or send `Accept: text/markdown`) to get its markdown twin.",
  "",
  "## Start here",
  "",
  ...promoted.map(bullet),
  "",
];

const sectionOrder = [
  "",
  "getting-started",
  "concepts",
  "using-programs",
  "using-stacks",
  "agent-skills",
  "building-stacks",
  "sdks",
  "cli",
  "a4-server",
];

const remainingKeys = [...grouped.keys()].sort(
  (a, b) =>
    (sectionOrder.indexOf(a) === -1 ? 99 : sectionOrder.indexOf(a)) -
    (sectionOrder.indexOf(b) === -1 ? 99 : sectionOrder.indexOf(b)),
);

for (const key of remainingKeys) {
  const items = grouped.get(key) ?? [];
  if (items.length === 0) continue;
  lines.push(`## ${SECTION_TITLES[key] ?? key}`, "", ...items.map(bullet), "");
}

lines.push(
  "## Optional",
  "",
  `- [Abridged documentation](${SITE}/llms-small.txt): compact documentation with non-essential content removed`,
  `- [Complete documentation](${SITE}/llms-full.txt): concatenates every documentation page`,
  `- [Agent bootstrap](${SITE}/agent.md): one-page setup instructions for coding agents`,
  `- [Legacy agent skill](${SITE}/skill.md): CLI-oriented operating guide for agents`,
  `- [Agent skills index](${SITE}/.well-known/agent-skills): well-known index of the five Arete product skills`,
  `- [Agent card](${SITE}/.well-known/agent-card.json): A2A-style agent card for this documentation site`,
  `- [Sitemap](${SITE}/sitemap.xml): XML sitemap of the HTML pages`,
  "",
  "The documentation MCP server is at https://docs.arete.run/mcp (tools: search_docs, fetch_page).",
  "",
);

const llmsPath = join(DIST, "llms.txt");
await writeFile(llmsPath, lines.join("\n"));
console.log(
  `[postbuild-agent-discovery] wrote ${pages.length} page links -> ${llmsPath}`,
);

const wellKnownDir = join(DIST, ".well-known");
await mkdir(wellKnownDir, { recursive: true });
await copyFile(llmsPath, join(wellKnownDir, "llms.txt"));
console.log(
  "[postbuild-agent-discovery] mirrored llms.txt -> dist/.well-known/llms.txt",
);

async function exists(path) {
  try {
    await readFile(path);
    return true;
  } catch {
    return false;
  }
}

const sitemap0 = join(DIST, "sitemap-0.xml");
const sitemapIndex = join(DIST, "sitemap-index.xml");
const sitemapXml = join(DIST, "sitemap.xml");
if (await exists(sitemap0)) {
  await copyFile(sitemap0, sitemapXml);
  console.log(
    "[postbuild-agent-discovery] copied sitemap-0.xml -> sitemap.xml",
  );
} else if (await exists(sitemapIndex)) {
  await copyFile(sitemapIndex, sitemapXml);
  console.log(
    "[postbuild-agent-discovery] copied sitemap-index.xml -> sitemap.xml",
  );
} else {
  console.warn("[postbuild-agent-discovery] no sitemap found to alias");
}
