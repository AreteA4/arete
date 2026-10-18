/**
 * Release registration for the handler host: recompute every hash the
 * release commits to, verify its review against the trusted review keys,
 * and only then load the handler bundle.
 */

import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

import { ProtocolContractError } from "../contracts/errors.js";
import { hashHandlerBundle, validateContract, type ProtocolContractKind } from "../contracts/index.js";
import { validateProtocolReleaseReview, verifyReleaseReview } from "../contracts/release.js";
import { decodeBase64 } from "../contracts/shape.js";
import type {
  ProtocolBuildReceiptV1,
  ProtocolReleaseReviewV1,
  ProtocolReleaseV1,
  ProtocolSpecV1,
} from "../contracts/types.js";
import { HostError, exactFields } from "./errors.js";

/** Trusted review keys: key id → 32-byte Ed25519 public key. */
export type ReviewKeys = ReadonlyMap<string, Uint8Array>;

/** A verified release held in memory. */
export interface LoadedRelease {
  releaseHash: string;
  specHash: string;
  bundleHash: string | null;
  spec: ProtocolSpecV1;
  /** The bundle's exports, or `null` for a release without handlers. */
  exports: Record<string, unknown> | null;
}

export interface VerifiedRelease {
  releaseHash: string;
  specHash: string;
  bundleHash: string | null;
  spec: ProtocolSpecV1;
  bundle: Uint8Array | null;
}

function contract(kind: ProtocolContractKind, field: string, value: unknown): string | null {
  try {
    return validateContract(kind, value);
  } catch (error) {
    if (error instanceof ProtocolContractError) {
      throw new HostError(400, "contract_invalid", `${field}: ${error.toString()}`);
    }
    throw error;
  }
}

function verifyReview(review: unknown, releaseHash: string, reviewKeys: ReviewKeys): ProtocolReleaseReviewV1 {
  let valid: ProtocolReleaseReviewV1;
  try {
    valid = validateProtocolReleaseReview(review);
  } catch (error) {
    if (error instanceof ProtocolContractError) {
      throw new HostError(403, "review_invalid", `review: ${error.toString()}`);
    }
    throw error;
  }
  if (valid.releaseHash !== releaseHash) {
    throw new HostError(403, "review_invalid", "the review is for a different release");
  }
  const key = reviewKeys.get(valid.reviewKeyId);
  if (key === undefined) {
    throw new HostError(403, "review_key_unknown", `review key '${valid.reviewKeyId}' is not trusted by this host`);
  }
  if (!verifyReleaseReview(valid, key)) {
    throw new HostError(403, "review_invalid", `the review signature does not verify under '${valid.reviewKeyId}'`);
  }
  return valid;
}

/**
 * Check a `PUT /handler-host/v1/releases/{releaseHash}` body. Every hash is
 * recomputed with the contract hashers; nothing in the body is trusted.
 */
export function verifyReleaseRequest(pathReleaseHash: string, body: unknown, reviewKeys: ReviewKeys): VerifiedRelease {
  const request = exactFields(body, "the request", ["release", "spec", "stackSource", "receipt", "review", "bundleBase64"]);

  const specHash = contract("protocol-spec", "spec", request.spec)!;
  const stackSourceHash = contract("protocol-stack-source", "stackSource", request.stackSource)!;
  const releaseHash = contract("protocol-release", "release", request.release)!;
  contract("protocol-build-receipt", "receipt", request.receipt);
  const release = request.release as ProtocolReleaseV1;
  const spec = request.spec as ProtocolSpecV1;
  const receipt = request.receipt as ProtocolBuildReceiptV1;

  if (releaseHash !== pathReleaseHash) {
    throw new HostError(400, "release_hash_mismatch", `the release hashes to ${releaseHash}, not ${pathReleaseHash}`);
  }
  if (release.specHash !== specHash) {
    throw new HostError(400, "spec_hash_mismatch", `the spec hashes to ${specHash}, not the release's ${release.specHash}`);
  }
  if (release.protocol !== spec.name || release.version !== spec.version) {
    throw new HostError(400, "spec_hash_mismatch", "the release names a different protocol or version than the spec");
  }
  if (release.stackSourceHash !== stackSourceHash) {
    throw new HostError(
      400,
      "stack_source_hash_mismatch",
      `the stack source hashes to ${stackSourceHash}, not the release's ${release.stackSourceHash}`,
    );
  }
  const outputs = receipt.outputs;
  const bundleHash = release.handlerBundleHash ?? null;
  if (
    outputs.releaseHash !== releaseHash ||
    outputs.specHash !== specHash ||
    outputs.stackSourceHash !== stackSourceHash ||
    (outputs.handlerBundleHash ?? null) !== bundleHash
  ) {
    throw new HostError(400, "receipt_mismatch", "the build receipt's outputs do not match the release");
  }

  // Review before the bundle is touched: unreviewed code never loads.
  verifyReview(request.review, releaseHash, reviewKeys);

  let bundle: Uint8Array | null = null;
  if (bundleHash === null) {
    if (request.bundleBase64 !== null) {
      throw new HostError(400, "bundle_unexpected", "the release has no handler bundle, so bundleBase64 must be null");
    }
  } else {
    if (typeof request.bundleBase64 !== "string") {
      throw new HostError(400, "bundle_missing", "the release has a handler bundle, so bundleBase64 is required");
    }
    const bytes = decodeBase64(request.bundleBase64);
    if (bytes === undefined) throw new HostError(400, "invalid_request", "bundleBase64 is not canonical base64");
    const actual = hashHandlerBundle(bytes);
    if (actual !== bundleHash) {
      throw new HostError(400, "bundle_hash_mismatch", `the bundle hashes to ${actual}, not the release's ${bundleHash}`);
    }
    bundle = bytes;
  }
  return { releaseHash, specHash, bundleHash, spec, bundle };
}

/** The handler export each handler step names, step → export. */
export function handlerExports(spec: ProtocolSpecV1): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [step, definition] of Object.entries(spec.steps)) {
    if (definition.build.kind === "handler") out[step] = definition.build.export;
  }
  return out;
}

function isHandler(value: unknown): boolean {
  if (typeof value === "function") return true;
  return value !== null && typeof value === "object" && typeof (value as { prepare?: unknown }).prepare === "function";
}

/** Loads bundles as ES modules from a private temporary directory. */
export class BundleLoader {
  private directory: string | undefined;
  private counter = 0;

  async load(verified: VerifiedRelease): Promise<LoadedRelease> {
    let exports: Record<string, unknown> | null = null;
    if (verified.bundle !== null) {
      this.directory ??= mkdtempSync(join(tmpdir(), "arete-handler-host-"));
      this.counter += 1;
      const file = join(this.directory, `bundle-${this.counter}.mjs`);
      writeFileSync(file, verified.bundle);
      try {
        exports = { ...((await import(pathToFileURL(file).href)) as Record<string, unknown>) };
      } catch (error) {
        throw new HostError(
          422,
          "bundle_load_failed",
          `the handler bundle failed to load: ${error instanceof Error ? error.message : String(error)}`,
        );
      } finally {
        rmSync(file, { force: true });
      }
    }
    for (const [step, name] of Object.entries(handlerExports(verified.spec))) {
      if (exports === null || !isHandler(exports[name])) {
        throw new HostError(422, "bundle_export_missing", `the bundle does not export handler '${name}' for step '${step}'`);
      }
    }
    return {
      releaseHash: verified.releaseHash,
      specHash: verified.specHash,
      bundleHash: verified.bundleHash,
      spec: verified.spec,
      exports,
    };
  }

  close(): void {
    if (this.directory !== undefined) rmSync(this.directory, { recursive: true, force: true });
    this.directory = undefined;
  }
}
