import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtemp, readdir, readFile, cp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { gunzipSync } from "node:zlib";
import {
  DISCOVERY_SCHEMA_V2,
  packAgentSkills,
  sha256Digest,
} from "./pack-agent-skills.mjs";

const PUBLIC_SKILLS = "public/.well-known/agent-skills";
const SKILL_NAMES = [
  "arete",
  "arete-streams",
  "arete-programs",
  "arete-stack-authoring",
  "arete-deploy",
];

test("well-known skill directories publish SKILL.md, not skill.md", async () => {
  for (const name of SKILL_NAMES) {
    const entries = await readdir(join(PUBLIC_SKILLS, name));
    assert.ok(
      entries.includes("SKILL.md"),
      `${name} is missing SKILL.md (found: ${entries.join(", ")})`,
    );
    assert.ok(
      !entries.includes("skill.md"),
      `${name} still has lowercase skill.md; Vercel is case-sensitive`,
    );
  }
});

test("packAgentSkills writes a v0.2 index with digest-verified archives", async () => {
  const tmp = await mkdtemp(join(tmpdir(), "arete-skills-"));
  await cp(PUBLIC_SKILLS, tmp, { recursive: true });
  const index = await packAgentSkills(tmp);

  assert.equal(index.$schema, DISCOVERY_SCHEMA_V2);
  assert.equal(index.skills.length, SKILL_NAMES.length);

  for (const entry of index.skills) {
    assert.equal(entry.type, "archive");
    assert.match(entry.digest, /^sha256:[a-f0-9]{64}$/);
    assert.equal(entry.url, `/.well-known/agent-skills/${entry.name}.tar.gz`);

    const archive = await readFile(join(tmp, `${entry.name}.tar.gz`));
    assert.equal(sha256Digest(archive), entry.digest);
    assert.equal(
      `sha256:${createHash("sha256").update(archive).digest("hex")}`,
      entry.digest,
    );

    const tar = gunzipSync(archive);
    const names = [];
    for (let offset = 0; offset + 512 <= tar.length; ) {
      const header = tar.subarray(offset, offset + 512);
      if (header.every((byte) => byte === 0)) break;
      const name = header.subarray(0, 100).toString("utf8").replace(/\0.*$/, "");
      const size = Number.parseInt(
        header.subarray(124, 135).toString("utf8").trim(),
        8,
      );
      names.push(name);
      offset += 512 + Math.ceil(size / 512) * 512;
    }
    assert.ok(names.includes("SKILL.md"), `${entry.name} archive missing SKILL.md`);
    assert.ok(
      !names.includes("skill.md"),
      `${entry.name} archive still has lowercase skill.md`,
    );
  }
});
