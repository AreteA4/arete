import {
  hashRawBytes,
  type ArtifactFileHash,
  type DecoderFixtureSetV2,
  type ProgramSpecV1,
  type PublicHashKind,
} from "@usearete/hash";

const kind: PublicHashKind = "artifact-file";
const artifactHash: ArtifactFileHash = hashRawBytes(kind, Uint8Array.from([1, 2, 3]));
const acceptsProgramSpec = (_value: ProgramSpecV1): void => {};
const acceptsDecoderFixture = (_value: DecoderFixtureSetV2): void => {};

void artifactHash;
void acceptsProgramSpec;
void acceptsDecoderFixture;
