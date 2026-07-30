"use strict";

const assert = require("node:assert/strict");
const hash = require("@usearete/hash");

for (const name of [
  "HASH_PROTOCOL_VERSION",
  "HashError",
  "hashArtifactTree",
  "hashRawBytes",
  "parseHashId",
  "parseIdlV1",
]) {
  assert(name in hash, `missing packed CommonJS export ${name}`);
}

const artifactHash = hash.hashRawBytes("artifact-file", Uint8Array.from([1, 2, 3]));
assert.equal(hash.parseHashId(artifactHash).kind, "artifact-file");
assert.equal(hash.HASH_PROTOCOL_VERSION, 1);
