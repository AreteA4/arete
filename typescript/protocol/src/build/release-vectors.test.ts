/**
 * The shared release-derivation vectors in `test-vectors/protocol-release/`.
 *
 * `server` cases run `deriveServerRelease` (what a hosted service derives
 * from an upload); `release` cases run `buildRelease` with an explicit
 * compiler identity and stack-source options, the way `arete-protocol build`
 * does. The Rust `derive_server_release` and `derive_release` run the same
 * cases and must produce the same documents and hashes.
 *
 * Set `UPDATE_RELEASE_VECTORS=1` to rewrite the expected outputs from this
 * implementation after an intended change.
 */

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { canonicalizeJcs, hashJcs } from "@usearete/hash";
import { describe, expect, it } from "vitest";

import { decodeBase64, hashHandlerBundle, parseContractJson, validateContract, validateProtocolSpec } from "../index.js";
import type { ProtocolSpecV1 } from "../contracts/types.js";
import { generateStackSource, type GenerateStackSourceOptions } from "../stack/generate.js";
import {
  buildRelease,
  compilerHash,
  deriveServerRelease,
  SERVER_COMPILER_NAME,
  SERVER_DERIVATION_VERSION,
  type CompilerIdentity,
} from "./artifacts.js";

const here = dirname(fileURLToPath(import.meta.url));
const vectorsDir = resolve(here, "../../../../test-vectors/protocol-release");
const vectorsPath = resolve(vectorsDir, "vectors.json");
const update = process.env.UPDATE_RELEASE_VECTORS === "1";
const decoder = new TextDecoder();

interface ReleaseVectorCase {
  name: string;
  spec: string;
  specOverrides?: Record<string, unknown>;
  bundleBase64?: string;
  mode: "server" | "release";
  compiler?: CompilerIdentity;
  stackOptions?: GenerateStackSourceOptions;
  expected?: Record<string, unknown>;
  error?: string;
}

interface ReleaseVectors {
  schema: string;
  serverCompiler?: { name: string; version: string; compilerHash: string };
  cases: ReleaseVectorCase[];
}

const vectors = JSON.parse(readFileSync(vectorsPath, "utf8")) as ReleaseVectors;

function specOf(vector: ReleaseVectorCase): ProtocolSpecV1 {
  const document = parseContractJson(readFileSync(resolve(vectorsDir, vector.spec))) as Record<string, unknown>;
  return validateProtocolSpec({ ...document, ...vector.specOverrides });
}

function text(value: unknown): string {
  return decoder.decode(canonicalizeJcs(value as never));
}

function run(vector: ReleaseVectorCase): { expected: Record<string, unknown> } | { error: string } {
  const spec = specOf(vector);
  const bundle = vector.bundleBase64 === undefined ? null : decodeBase64(vector.bundleBase64)!;
  if (vector.mode === "server") {
    try {
      const derived = deriveServerRelease(spec, bundle);
      expect(validateContract("protocol-release", derived.release)).toBe(derived.releaseHash);
      expect(validateContract("protocol-build-receipt", derived.receipt)).toBeNull();
      expect(validateContract("protocol-stack-source", derived.stackSource)).toBe(derived.stackSourceHash);
      return {
        expected: {
          stackSourceHash: derived.stackSourceHash,
          releaseHash: derived.releaseHash,
          release: text(derived.release),
          receipt: text(derived.receipt),
        },
      };
    } catch (error) {
      return { error: (error as Error).message };
    }
  }
  const stackSource = generateStackSource(spec, vector.stackOptions ?? {});
  const stackSourceHash = hashJcs("protocol-stack-source", stackSource as never);
  const release = buildRelease(spec, {
    specHash: hashJcs("protocol-spec", spec as never),
    handlerBundleHash: bundle === null ? null : hashHandlerBundle(bundle),
    stackSourceHash,
    compilerHash: compilerHash(vector.compiler!),
  });
  return {
    expected: {
      stackSourceHash,
      releaseHash: hashJcs("protocol-release", release as never),
      release: text(release),
    },
  };
}

describe("protocol release vectors", () => {
  it("pins the server compiler identity", () => {
    expect(vectors.schema).toBe("arete.protocol-release-vectors/v1");
    const serverCompiler = {
      name: SERVER_COMPILER_NAME,
      version: SERVER_DERIVATION_VERSION,
      compilerHash: compilerHash({ name: SERVER_COMPILER_NAME, version: SERVER_DERIVATION_VERSION, esbuild: "0.0.0" }),
    };
    if (update) vectors.serverCompiler = serverCompiler;
    else expect(vectors.serverCompiler).toEqual(serverCompiler);
    expect(new Set(vectors.cases.map((vector) => vector.name)).size).toBe(vectors.cases.length);
  });

  for (const vector of vectors.cases) {
    it(vector.name, () => {
      const result = run(vector);
      if (update) {
        delete vector.expected;
        delete vector.error;
        if ("error" in result) vector.error = result.error;
        else vector.expected = result.expected;
        return;
      }
      if ("error" in result) {
        expect(vector.expected).toBeUndefined();
        expect(result.error).toBe(vector.error);
      } else {
        expect(vector.error).toBeUndefined();
        expect(result.expected).toEqual(vector.expected);
      }
    });
  }

  it("differs from a local build only through the compiler identity and stack source", () => {
    const local = vectors.cases.find((vector) => vector.name === "token-raise.local-build")!.expected!;
    const server = vectors.cases.find((vector) => vector.name === "token-raise.server")!.expected!;
    const strip = (release: unknown) => {
      const parsed = JSON.parse(release as string) as Record<string, unknown>;
      delete parsed.compilerHash;
      delete parsed.stackSourceHash;
      return parsed;
    };
    expect(strip(local.release)).toEqual(strip(server.release));
    expect(local.releaseHash).not.toBe(server.releaseHash);
  });

  if (update) {
    it("writes the vector file", () => {
      writeFileSync(vectorsPath, `${JSON.stringify(vectors, null, 2)}\n`);
    });
  }
});
