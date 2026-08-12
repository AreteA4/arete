import assert from "node:assert/strict";
import * as hash from "@usearete/hash";

for (const name of [
  "HASH_PROTOCOL_VERSION",
  "HashError",
  "hashArtifactTree",
  "hashRawBytes",
  "hashDecoderFixtureSetV2",
  "parseHashId",
  "parseIdlV1",
  "validateHostedManagedProgramReleaseV2",
]) {
  assert(name in hash, `missing packed ESM export ${name}`);
}

const artifactHash = hash.hashRawBytes("artifact-file", Uint8Array.from([1, 2, 3]));
assert.equal(hash.parseHashId(artifactHash).kind, "artifact-file");
assert.equal(hash.HASH_PROTOCOL_VERSION, 1);
