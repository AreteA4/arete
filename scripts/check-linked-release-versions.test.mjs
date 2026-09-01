import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  validateLinkedReleaseVersions,
} from "./check-linked-release-versions.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const config = {
  packages: {
    alpha: { "release-type": "rust", component: "alpha" },
    beta: { "release-type": "node", component: "beta" },
    gamma: { "release-type": "python", component: "gamma" },
  },
  plugins: [
    {
      type: "linked-versions",
      groupName: "product",
      components: ["alpha", "beta", "gamma"],
    },
  ],
};

test("accepts synchronized manifest and source versions", () => {
  const result = validateLinkedReleaseVersions(
    config,
    { alpha: "1.2.3", beta: "1.2.3", gamma: "1.2.3" },
    { alpha: "1.2.3", beta: "1.2.3", gamma: "1.2.3" },
  );

  assert.deepEqual(result.errors, []);
  assert.deepEqual(result.summaries, ["product: 3 components at 1.2.3"]);
});

test("rejects a partially bumped linked group", () => {
  const result = validateLinkedReleaseVersions(config, {
    alpha: "1.2.4",
    beta: "1.2.3",
    gamma: "1.2.3",
  });

  assert.equal(result.errors.length, 1);
  assert.match(
    result.errors[0],
    /alpha=1\.2\.4, beta=1\.2\.3, gamma=1\.2\.3/,
  );
});

test("rejects source and release manifest drift", () => {
  const result = validateLinkedReleaseVersions(
    config,
    { alpha: "1.2.3", beta: "1.2.3", gamma: "1.2.3" },
    { alpha: "1.2.4", beta: "1.2.3", gamma: "1.2.3" },
  );

  assert.deepEqual(result.errors, [
    "alpha source version 1.2.4 does not match manifest version 1.2.3",
  ]);
});

test("rejects an unknown linked component", () => {
  const invalidConfig = structuredClone(config);
  invalidConfig.plugins[0].components.push("missing");

  const result = validateLinkedReleaseVersions(invalidConfig, {
    alpha: "1.2.3",
    beta: "1.2.3",
    gamma: "1.2.3",
  });

  assert.deepEqual(result.errors, [
    "linked group product references unconfigured component missing",
  ]);
});

test("keeps the repository Python SDK in the arete linked group", () => {
  const repositoryConfig = JSON.parse(
    fs.readFileSync(path.join(root, "release-please-config.json"), "utf8"),
  );
  const linkedGroup = repositoryConfig.plugins.find(
    (plugin) => plugin.type === "linked-versions" && plugin.groupName === "arete",
  );

  assert.equal(
    repositoryConfig.packages["python/arete-sdk"].component,
    "arete-python",
  );
  assert.ok(linkedGroup.components.includes("arete-python"));
});
