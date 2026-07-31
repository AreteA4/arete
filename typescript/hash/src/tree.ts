import { framedTuplePayload, hashRawBytes } from "./canonical.js";
import { hashError } from "./error.js";
import { concatBytes, hashCanonicalPayload, u64be } from "./hash.js";
import type {
  ArtifactTreeEntry,
  ArtifactTreeHashKind,
  HashId,
} from "./types.js";

const encoder = new TextEncoder();

export function validateArtifactPath(path: string): void {
  const invalid = (reason: string): never =>
    hashError("invalid-artifact-path", `invalid artifact path '${path}': ${reason}`);
  if (path.length === 0) invalid("path must contain at least one segment");
  if (path.startsWith("/") || path.endsWith("/")) {
    invalid("leading and trailing slashes are forbidden");
  }
  if (path.includes("//")) invalid("repeated slashes are forbidden");
  if (path.includes("\\")) invalid("backslashes are forbidden");
  if (path.includes("\0")) invalid("NUL bytes are forbidden");
  if (path.split("/").some((segment) => segment === "" || segment === "." || segment === "..")) {
    invalid("empty, '.' and '..' segments are forbidden");
  }
}

export function artifactTreePayload(entries: readonly ArtifactTreeEntry[]): Uint8Array {
  const sorted = [...entries];
  for (const entry of sorted) {
    validateArtifactPath(entry.path);
    if (entry.type === "symlink") {
      return hashError(
        "symlink-artifact",
        `artifact tree entries cannot be symlinks: '${entry.path}'`,
      );
    }
  }
  sorted.sort((left, right) => compareBytes(encoder.encode(left.path), encoder.encode(right.path)));

  const seen = new Set<string>();
  const payload: Uint8Array[] = [u64be(sorted.length)];
  for (const entry of sorted) {
    if (seen.has(entry.path)) {
      return hashError("duplicate-artifact-path", `duplicate artifact path '${entry.path}'`);
    }
    seen.add(entry.path);
    if (entry.type === "symlink") throw new Error("unreachable");
    const fileHash = hashRawBytes("artifact-file", entry.bytes);
    payload.push(
      framedTuplePayload([
        { label: "path", value: encoder.encode(entry.path) },
        { label: "fileHash", value: encoder.encode(fileHash) },
      ]),
    );
  }
  return concatBytes(...payload);
}

export function hashArtifactTree<K extends ArtifactTreeHashKind>(
  kind: K,
  entries: readonly ArtifactTreeEntry[],
): HashId<K> {
  return hashCanonicalPayload(kind, "artifact-tree-v1", artifactTreePayload(entries));
}

function compareBytes(left: Uint8Array, right: Uint8Array): number {
  const length = Math.min(left.length, right.length);
  for (let index = 0; index < length; index += 1) {
    const difference = (left[index] ?? 0) - (right[index] ?? 0);
    if (difference !== 0) return difference;
  }
  return left.length - right.length;
}
