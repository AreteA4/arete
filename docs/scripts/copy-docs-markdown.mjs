/**
 * After `astro build`, mirror each content page as a .md file next to HTML routes
 * so URLs like /using-stacks/quickstart.md resolve for agents (AFDocs markdown-url-support).
 * Strips YAML frontmatter only; body matches authored MDX/Markdown source.
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.join(__dirname, "..");
const docsDir = path.join(root, "src", "content", "docs");
const distDir = path.join(root, "dist");

function walkMdx(dir, out = []) {
  for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, ent.name);
    if (ent.isDirectory()) walkMdx(full, out);
    else if (ent.name.endsWith(".mdx")) out.push(full);
  }
  return out;
}

function stripFrontmatter(src) {
  return src.replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n?/, "");
}

if (!fs.existsSync(distDir)) {
  console.error("copy-docs-markdown: dist/ missing — run astro build first");
  process.exit(1);
}

const files = walkMdx(docsDir);
let n = 0;
for (const file of files) {
  const rel = path.relative(docsDir, file);
  const outPath = path.join(distDir, rel.replace(/\.mdx$/, ".md"));
  fs.mkdirSync(path.dirname(outPath), { recursive: true });
  const body = stripFrontmatter(fs.readFileSync(file, "utf8"));
  fs.writeFileSync(outPath, body, "utf8");
  n++;
}
console.log(`copy-docs-markdown: wrote ${n} .md mirrors under dist/`);
