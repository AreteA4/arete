import { readFileSync } from "node:fs";
import { expect, test } from "vitest";
import { createSdkDefinitionV2, hashSdkDefinitionV2, type SdkDefinitionV2, type SdkOutputTreeHash, type ProgramSpecHash } from "./index.js";

test("SDK definition V2 matches the shared Rust vector and tracks content contract", () => {
  const vector = JSON.parse(readFileSync(new URL("../../../test-vectors/sdk-definition-v2.json", import.meta.url), "utf8"));
  const definition = vector.projection as SdkDefinitionV2;
  expect(createSdkDefinitionV2(definition.inputHash, definition.target, definition.runtimeContract, definition.outputTreeHash)).toEqual(definition);
  expect(hashSdkDefinitionV2(definition)).toBe(vector.expectedHash);
  for (const changed of [
    { ...definition, outputTreeHash: `arete:h1:sdk-output-tree:sha256:${"03".repeat(32)}` as SdkOutputTreeHash },
    { ...definition, inputHash: `arete:h1:program-spec:sha256:${"04".repeat(32)}` as ProgramSpecHash },
    { ...definition, runtimeContract: "@usearete/sdk/program-definition-v2" },
    { ...definition, target: "rust" as const },
  ]) expect(hashSdkDefinitionV2(changed)).not.toBe(vector.expectedHash);
  for (const runtimeContract of ["", "has space", "nonascii-é"]) {
    expect(() => hashSdkDefinitionV2({ ...definition, runtimeContract })).toThrow();
  }
  for (const changed of [
    { ...definition, compilerHash: "ignored?" },
    { ...definition, target: "unknown" },
    { ...definition, schema: "arete.sdk-definition/v3" },
    { ...definition, outputTreeHash: definition.inputHash },
  ]) expect(() => hashSdkDefinitionV2(changed as SdkDefinitionV2)).toThrow();
});
