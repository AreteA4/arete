/**
 * Release-side contracts: the generated stack source, the release, its
 * review signature, the build receipt and published resources. Mirrors
 * `arete-hash/src/protocol/release.rs`.
 */

import { ed25519 } from "@noble/curves/ed25519";

import {
  PROTOCOL_BUILD_RECEIPT_SCHEMA_V1,
  PROTOCOL_RELEASE_REVIEW_SCHEMA_V1,
  PROTOCOL_RELEASE_SCHEMA_V1,
  PROTOCOL_RESOURCE_SCHEMA_V1,
  PROTOCOL_STACK_SOURCE_SCHEMA_V1,
  STACK_SOURCE_SCHEMA_V1,
  type ProtocolBuildReceiptV1,
  type ProtocolReleaseReviewV1,
  type ProtocolReleaseV1,
  type ProtocolResourceV1,
  type ProtocolStackSourceV1,
  type TrustTier,
} from "./types.js";
import {
  Obj,
  asStr,
  boundedStr,
  decodeBase64,
  err,
  identifierValue,
  isObject,
  oneOf,
  protocolName,
  pubkey,
  rejectFloats,
  schemaTag,
  semver,
  sha256Hex,
  sortedEntries,
} from "./shape.js";
import { hashIdValue } from "./spec.js";

const encoder = new TextEncoder();

// ---------------------------------------------------------------------------
// Stack source
// ---------------------------------------------------------------------------

export function validateProtocolStackSource(value: unknown): ProtocolStackSourceV1 {
  rejectFloats(value, "");
  const root = new Obj(value, "", ["schema", "generator", "profile", "stackSource"]);
  schemaTag(root.req("schema"), "/schema", PROTOCOL_STACK_SOURCE_SCHEMA_V1);
  shapeGenerator(root.req("generator"), "/generator");
  oneOf(root.req("profile"), "/profile", ["interim", "full"]);
  const source = root.req("stackSource");
  if (!isObject(source)) err("invalid-type", "/stackSource", "expected an object");
  if (!Object.prototype.hasOwnProperty.call(source, "schema")) {
    err("missing-field", "/stackSource/schema", "missing field 'schema'");
  }
  schemaTag(source.schema, "/stackSource/schema", STACK_SOURCE_SCHEMA_V1);
  return value as ProtocolStackSourceV1;
}

function shapeGenerator(value: unknown, path: string): void {
  const generator = new Obj(value, path, ["name", "version"]);
  const name = boundedStr(generator.req("name"), generator.pathOf("name"), 128);
  if (name.length === 0) err("invalid-value", generator.pathOf("name"), "name must not be empty");
  semver(generator.req("version"), generator.pathOf("version"));
}

// ---------------------------------------------------------------------------
// Release
// ---------------------------------------------------------------------------

export function validateProtocolRelease(value: unknown): ProtocolReleaseV1 {
  rejectFloats(value, "");
  const root = new Obj(value, "", [
    "schema",
    "protocol",
    "version",
    "specHash",
    "handlerBundleHash",
    "stackSourceHash",
    "compilerHash",
    "dependencies",
  ]);
  schemaTag(root.req("schema"), "/schema", PROTOCOL_RELEASE_SCHEMA_V1);
  protocolName(asStr(root.req("protocol"), "/protocol"), "/protocol");
  semver(root.req("version"), "/version");
  hashIdValue(root.req("specHash"), "/specHash", "protocol-spec");
  const bundle = root.nullable("handlerBundleHash");
  if (bundle !== undefined) hashIdValue(bundle, "/handlerBundleHash", "protocol-handler-bundle");
  hashIdValue(root.req("stackSourceHash"), "/stackSourceHash", "protocol-stack-source");
  hashIdValue(root.req("compilerHash"), "/compilerHash", "compiler");
  const deps = new Obj(root.req("dependencies"), "/dependencies", ["programs", "stacks", "sdkExtensions"]);
  sortedEntries(deps.req("programs"), "/dependencies/programs", (item, path) => {
    const entry = new Obj(item, path, ["alias", "programId", "programSpecHash", "programReleaseHash"]);
    const alias = identifierValue(entry.req("alias"), entry.pathOf("alias"));
    pubkey(entry.req("programId"), entry.pathOf("programId"));
    hashIdValue(entry.req("programSpecHash"), entry.pathOf("programSpecHash"), "program-spec");
    const release = entry.opt("programReleaseHash");
    if (release !== undefined) {
      hashIdValue(release, entry.pathOf("programReleaseHash"), "program-release");
    }
    return alias;
  });
  sortedEntries(deps.req("stacks"), "/dependencies/stacks", (item, path) => {
    const entry = new Obj(item, path, ["alias", "stackManifestHash"]);
    const alias = identifierValue(entry.req("alias"), entry.pathOf("alias"));
    hashIdValue(entry.req("stackManifestHash"), entry.pathOf("stackManifestHash"), "stack-manifest");
    return alias;
  });
  sortedEntries(deps.req("sdkExtensions"), "/dependencies/sdkExtensions", (item, path) => {
    const entry = new Obj(item, path, ["alias", "sdkExtensionHash"]);
    const alias = identifierValue(entry.req("alias"), entry.pathOf("alias"));
    hashIdValue(entry.req("sdkExtensionHash"), entry.pathOf("sdkExtensionHash"), "sdk-extension");
    return alias;
  });
  return value as ProtocolReleaseV1;
}

// ---------------------------------------------------------------------------
// Review
// ---------------------------------------------------------------------------

/**
 * The exact bytes a reviewer signs:
 * `arete.protocol-release-review/v1\n<keyId>\n<trustTier>\n<releaseHash>`.
 * Hosts and the API verify against these bytes with the same public keys.
 */
export function reviewSigningMessage(
  releaseHash: string,
  reviewKeyId: string,
  trustTier: TrustTier,
): Uint8Array {
  return encoder.encode(
    `${PROTOCOL_RELEASE_REVIEW_SCHEMA_V1}\n${reviewKeyId}\n${trustTier}\n${releaseHash}`,
  );
}

/**
 * Validate a release review and verify its Ed25519 signature against a
 * 32-byte public key. Returns `false` for a well-formed review whose
 * signature does not verify; throws `ProtocolContractError` for a malformed
 * review.
 */
export function verifyReleaseReview(review: unknown, publicKey: Uint8Array): boolean {
  const valid = validateProtocolReleaseReview(review);
  if (publicKey.length !== 32) return false;
  const signature = decodeBase64(valid.signature)!;
  const message = reviewSigningMessage(valid.releaseHash, valid.reviewKeyId, valid.trustTier);
  try {
    return ed25519.verify(signature, message, publicKey);
  } catch {
    return false;
  }
}

export function validateProtocolReleaseReview(value: unknown): ProtocolReleaseReviewV1 {
  rejectFloats(value, "");
  const root = new Obj(value, "", ["schema", "releaseHash", "reviewKeyId", "trustTier", "signature"]);
  schemaTag(root.req("schema"), "/schema", PROTOCOL_RELEASE_REVIEW_SCHEMA_V1);
  hashIdValue(root.req("releaseHash"), "/releaseHash", "protocol-release");
  keyId(root.req("reviewKeyId"), "/reviewKeyId");
  oneOf(root.req("trustTier"), "/trustTier", ["reviewed"]);
  const signature = asStr(root.req("signature"), "/signature");
  const bytes = decodeBase64(signature);
  if (bytes === undefined || bytes.length !== 64) {
    err("invalid-signature", "/signature", "expected a base64 64-byte Ed25519 signature");
  }
  return value as ProtocolReleaseReviewV1;
}

/** `^[a-z0-9][a-z0-9-]{0,63}$`. */
export function keyId(value: unknown, path: string): string {
  const text = asStr(value, path);
  if (!/^[a-z0-9][a-z0-9-]{0,63}$/.test(text)) {
    err("invalid-identifier", path, "key ids match ^[a-z0-9][a-z0-9-]{0,63}$");
  }
  return text;
}

// ---------------------------------------------------------------------------
// Build receipt
// ---------------------------------------------------------------------------

export function validateProtocolBuildReceipt(value: unknown): ProtocolBuildReceiptV1 {
  rejectFloats(value, "");
  const root = new Obj(value, "", [
    "schema",
    "sourceTreeSha256",
    "lockfileSha256",
    "dependencies",
    "compiler",
    "outputs",
  ]);
  schemaTag(root.req("schema"), "/schema", PROTOCOL_BUILD_RECEIPT_SCHEMA_V1);
  sha256Hex(root.req("sourceTreeSha256"), "/sourceTreeSha256");
  sha256Hex(root.req("lockfileSha256"), "/lockfileSha256");
  sortedEntries(root.req("dependencies"), "/dependencies", (item, path) => {
    const entry = new Obj(item, path, ["name", "version", "integrity"]);
    const name = boundedStr(entry.req("name"), entry.pathOf("name"), 214);
    boundedStr(entry.req("version"), entry.pathOf("version"), 64);
    boundedStr(entry.req("integrity"), entry.pathOf("integrity"), 256);
    return name;
  });
  const compiler = new Obj(root.req("compiler"), "/compiler", ["name", "version", "esbuild"]);
  boundedStr(compiler.req("name"), "/compiler/name", 128);
  semver(compiler.req("version"), "/compiler/version");
  semver(compiler.req("esbuild"), "/compiler/esbuild");
  const outputs = new Obj(root.req("outputs"), "/outputs", [
    "specHash",
    "handlerBundleHash",
    "stackSourceHash",
    "releaseHash",
  ]);
  hashIdValue(outputs.req("specHash"), "/outputs/specHash", "protocol-spec");
  const bundle = outputs.nullable("handlerBundleHash");
  if (bundle !== undefined) hashIdValue(bundle, "/outputs/handlerBundleHash", "protocol-handler-bundle");
  hashIdValue(outputs.req("stackSourceHash"), "/outputs/stackSourceHash", "protocol-stack-source");
  hashIdValue(outputs.req("releaseHash"), "/outputs/releaseHash", "protocol-release");
  return value as ProtocolBuildReceiptV1;
}

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

export function validateProtocolResource(value: unknown): ProtocolResourceV1 {
  rejectFloats(value, "");
  const root = new Obj(value, "", ["schema", "name", "schemaSha256", "value"]);
  schemaTag(root.req("schema"), "/schema", PROTOCOL_RESOURCE_SCHEMA_V1);
  identifierValue(root.req("name"), "/name");
  sha256Hex(root.req("schemaSha256"), "/schemaSha256");
  root.req("value");
  return value as ProtocolResourceV1;
}
