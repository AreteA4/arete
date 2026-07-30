import {
  hashRawBytes,
  type ArtifactFileHash,
  type ProgramSpecV1,
  type PublicHashKind,
} from "@usearete/hash";

// @ts-expect-error Decoder fixture DTOs are intentionally private package internals.
import type { DecoderFixtureSetV1 } from "@usearete/hash";

const kind: PublicHashKind = "artifact-file";
const artifactHash: ArtifactFileHash = hashRawBytes(kind, Uint8Array.from([1, 2, 3]));
const acceptsProgramSpec = (_value: ProgramSpecV1): void => {};

void artifactHash;
void acceptsProgramSpec;
