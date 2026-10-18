/**
 * The shared protocol stack vectors in `test-vectors/protocol-stack/`.
 *
 * Every case names a reference spec (optionally with top-level fields
 * replaced), the generator options, and either the exact canonical output
 * bytes with their hash or the error message. The Rust generator runs the
 * same cases and must produce the same bytes.
 *
 * Set `UPDATE_STACK_VECTORS=1` to rewrite the expected outputs from this
 * generator after an intended change.
 */

import { existsSync, readFileSync, unlinkSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { canonicalizeJcs } from "@usearete/hash";
import { describe, expect, it } from "vitest";

import { parseContractJson, validateContract, validateProtocolSpec } from "../index.js";
import type { ProtocolStackSourceV1 } from "../contracts/types.js";
import { PACKAGE_NAME } from "../version.js";
import { generateStackSource, StackGenerationError, type GenerateStackSourceOptions } from "./generate.js";

const here = dirname(fileURLToPath(import.meta.url));
const vectorsDir = resolve(here, "../../../../test-vectors/protocol-stack");
const vectorsPath = resolve(vectorsDir, "vectors.json");
const update = process.env.UPDATE_STACK_VECTORS === "1";
const decoder = new TextDecoder();

interface StackVectorCase {
  name: string;
  spec: string;
  specOverrides?: Record<string, unknown>;
  options: GenerateStackSourceOptions;
  output?: string;
  hash?: string;
  error?: string;
}

interface StackVectors {
  schema: string;
  generator: { name: string; version: string };
  cases: StackVectorCase[];
}

const vectors = JSON.parse(readFileSync(vectorsPath, "utf8")) as StackVectors;

function specOf(vector: StackVectorCase) {
  const document = parseContractJson(readFileSync(resolve(vectorsDir, vector.spec))) as Record<string, unknown>;
  return validateProtocolSpec({ ...document, ...vector.specOverrides });
}

/** Run a case; the output carries the vector file's generator identity. */
function run(vector: StackVectorCase): { bytes: string; hash: string } | { error: string } {
  const spec = specOf(vector);
  let generated: ProtocolStackSourceV1;
  try {
    generated = generateStackSource(spec, vector.options);
  } catch (error) {
    if (error instanceof StackGenerationError) return { error: error.message };
    throw error;
  }
  expect(generated.generator.name).toBe(PACKAGE_NAME);
  const pinned = { ...generated, generator: vectors.generator };
  const hash = validateContract("protocol-stack-source", pinned);
  return { bytes: decoder.decode(canonicalizeJcs(pinned as never)), hash: hash! };
}

describe("protocol stack vectors", () => {
  it("has unique case names", () => {
    expect(vectors.schema).toBe("arete.protocol-stack-vectors/v1");
    expect(vectors.generator.name).toBe(PACKAGE_NAME);
    expect(new Set(vectors.cases.map((vector) => vector.name)).size).toBe(vectors.cases.length);
  });

  for (const vector of vectors.cases) {
    it(vector.name, () => {
      const result = run(vector);
      if (update) {
        delete vector.output;
        delete vector.hash;
        delete vector.error;
        const file = `${vector.name}.protocol-stack-source.json`;
        if ("error" in result) {
          vector.error = result.error;
          if (existsSync(resolve(vectorsDir, file))) unlinkSync(resolve(vectorsDir, file));
        } else {
          vector.output = file;
          vector.hash = result.hash;
          writeFileSync(resolve(vectorsDir, file), result.bytes);
        }
        return;
      }
      if ("error" in result) {
        expect(vector.output).toBeUndefined();
        expect(result.error).toBe(vector.error);
      } else {
        expect(vector.error).toBeUndefined();
        expect(result.bytes).toBe(readFileSync(resolve(vectorsDir, vector.output!), "utf8"));
        expect(result.hash).toBe(vector.hash);
      }
    });
  }

  it("covers the token-raise golden files", () => {
    const golden = (file: string) =>
      decoder.decode(canonicalizeJcs(JSON.parse(readFileSync(resolve(here, "testdata", file), "utf8"))));
    const output = (name: string) =>
      readFileSync(resolve(vectorsDir, vectors.cases.find((vector) => vector.name === name)!.output!), "utf8");
    expect(output("token-raise.interim")).toBe(golden("token-raise.protocol-stack-source.json"));
    expect(output("token-raise.full")).toBe(golden("token-raise.full.protocol-stack-source.json"));
  });

  if (update) {
    it("writes the vector file", () => {
      writeFileSync(vectorsPath, `${JSON.stringify(vectors, null, 2)}\n`);
    });
  }
});
