#!/usr/bin/env node
// Build v0.2 well-known skill archives and index.json.
// Archives are reproducible (ustar + gzip with zeroed mtime/OS) so digests
// stay stable across machines. Files sit at the archive root, not in a
// wrapper directory.

import { createHash } from "node:crypto";
import { readdir, readFile, writeFile } from "node:fs/promises";
import { join, relative } from "node:path";
import { gzipSync } from "node:zlib";
import { fileURLToPath } from "node:url";

export const DISCOVERY_SCHEMA_V2 =
  "https://schemas.agentskills.io/discovery/0.2.0/schema.json";

const SKILL_NAMES = [
  "arete",
  "arete-streams",
  "arete-programs",
  "arete-stack-authoring",
  "arete-deploy",
];

function octal(num, width) {
  return `${num.toString(8).padStart(width - 1, "0")}\0`;
}

function tarHeader(name, size) {
  if (Buffer.byteLength(name) > 99) {
    throw new Error(`ustar name too long: ${name}`);
  }
  const buf = Buffer.alloc(512, 0);
  buf.write(name, 0);
  Buffer.from(octal(0o644, 8)).copy(buf, 100);
  Buffer.from(octal(0, 8)).copy(buf, 108);
  Buffer.from(octal(0, 8)).copy(buf, 116);
  Buffer.from(octal(size, 12)).copy(buf, 124);
  Buffer.from(octal(0, 12)).copy(buf, 136);
  Buffer.from("        ").copy(buf, 148);
  buf[156] = 0x30;
  Buffer.from("ustar\0").copy(buf, 257);
  Buffer.from("00").copy(buf, 263);
  let sum = 0;
  for (const byte of buf) sum += byte;
  Buffer.from(`${sum.toString(8).padStart(6, "0")}\0 `).copy(buf, 148);
  return buf;
}

export function createTarGz(files) {
  const sorted = [...files].sort((a, b) => a.name.localeCompare(b.name));
  const parts = [];
  for (const { name, content } of sorted) {
    const body = Buffer.isBuffer(content) ? content : Buffer.from(content);
    parts.push(tarHeader(name, body.length));
    parts.push(body);
    const pad = (512 - (body.length % 512)) % 512;
    if (pad) parts.push(Buffer.alloc(pad));
  }
  parts.push(Buffer.alloc(1024));
  const gzipped = gzipSync(Buffer.concat(parts), { level: 9 });
  gzipped.writeUInt32LE(0, 4);
  gzipped[8] = 0;
  gzipped[9] = 255;
  return gzipped;
}

export function sha256Digest(bytes) {
  return `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
}

function parseDescription(markdown) {
  const match = markdown.match(/^---\n([\s\S]*?)\n---/);
  if (!match) throw new Error("SKILL.md is missing YAML frontmatter");
  const line = match[1]
    .split("\n")
    .find((entry) => entry.startsWith("description:"));
  if (!line) throw new Error("SKILL.md is missing a description");
  return line.slice("description:".length).trim().replace(/^"|"$/g, "");
}

async function collectSkillFiles(dir) {
  const files = [];
  async function walk(current) {
    const entries = await readdir(current, { withFileTypes: true });
    entries.sort((a, b) => a.name.localeCompare(b.name));
    for (const entry of entries) {
      const full = join(current, entry.name);
      if (entry.isDirectory()) {
        await walk(full);
        continue;
      }
      if (!entry.isFile()) continue;
      if (entry.name.endsWith(".tar.gz")) continue;
      if (entry.name === "index.json") continue;
      files.push({
        name: relative(dir, full).replaceAll("\\", "/"),
        content: await readFile(full),
      });
    }
  }
  await walk(dir);
  if (!files.some((file) => file.name === "SKILL.md")) {
    throw new Error(`${dir} is missing SKILL.md`);
  }
  if (files.some((file) => file.name === "skill.md")) {
    throw new Error(`${dir} still has lowercase skill.md`);
  }
  return files;
}

export async function packAgentSkills(root) {
  const skills = [];
  for (const name of SKILL_NAMES) {
    const dir = join(root, name);
    const files = await collectSkillFiles(dir);
    const skillMd = files.find((file) => file.name === "SKILL.md");
    const archive = createTarGz(files);
    const archiveName = `${name}.tar.gz`;
    await writeFile(join(root, archiveName), archive);
    skills.push({
      name,
      type: "archive",
      description: parseDescription(skillMd.content.toString("utf8")),
      url: `/.well-known/agent-skills/${archiveName}`,
      digest: sha256Digest(archive),
    });
  }

  const index = {
    $schema: DISCOVERY_SCHEMA_V2,
    skills,
  };
  await writeFile(join(root, "index.json"), `${JSON.stringify(index, null, 2)}\n`);
  return index;
}

const invokedDirectly = process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1];
if (invokedDirectly) {
  const root = process.argv[2] || "public/.well-known/agent-skills";
  const index = await packAgentSkills(root);
  console.log(
    `[pack-agent-skills] wrote ${index.skills.length} archives + v0.2 index -> ${root}`,
  );
}
