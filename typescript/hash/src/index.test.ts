import { readFileSync } from "node:fs";

import { describe, expect, expectTypeOf, test } from "vitest";

import {
  DECODER_FIXTURE_ACCOUNT_DECODE_ERROR_CATEGORIES,
  DECODER_FIXTURE_MAX_ACCOUNT_BYTES,
  DECODER_FIXTURE_MAX_CASES,
  DECODER_FIXTURE_MAX_TOTAL_ACCOUNT_BYTES,
  hashDecoderFixtureSetV2,
  validateDecoderFixtureSetV2,
} from "./fixture.js";

import {
  CANONICALIZATION_PROFILES,
  HASH_ALGORITHM,
  HASH_PROTOCOL_LABEL,
  HASH_PROTOCOL_VERSION,
  IDENTITY_REGISTRY,
  NON_HASH_IDENTITY_REGISTRY,
  artifactTreePayload,
  bytesToHex,
  canonicalizeJcs,
  canonicalizeJsonBytes,
  framedPreimage,
  framedTuplePayload,
  createCompilerV1,
  createSdkDefinitionV1,
  hashArtifactTree,
  hashFramedTuple,
  hashCompilerV1,
  hashHostedManagedProgramReleaseV2,
  hashJcs,
  hashJsonBytes,
  hashOssGeneratedProgramReleaseV1,
  hashProgramSpecV1,
  hashRawBytes,
  hashSdkDefinitionV1,
  hexToBytes,
  isProgramReadBindingId,
  parseCanonicalizationProfile,
  parseHashId,
  parseProgramReadBindingId,
  parseIdlV1,
  projectIdlV1,
  projectWithoutArtifactHash,
  type ArtifactTreeEntry,
  type AuthenticatedOwnerHashKind,
  validateHostedManagedProgramReleaseV2,
  type HostedManagedProgramReleaseV2,
  type JsonValue,
  type OssGeneratedProgramReleaseV1,
  type ProgramSpecV1,
  type ProgramSpecHash,
  type PublicHashKind,
  type RawBytesHashKind,
  type TupleField,
} from "./index.js";

test("projects conflicting instruction-local PDAs inline", () => {
  const source = {
    address: "11111111111111111111111111111111",
    metadata: { name: "pda_conflict", version: "0.1.0" },
    instructions: [
      {
        name: "initialize",
        discriminator: [1],
        accounts: [
          {
            name: "launch",
            pda: {
              seeds: [
                { kind: "const", value: [108, 97, 117, 110, 99, 104] },
                { kind: "account", path: "admin" },
              ],
            },
          },
        ],
        args: [],
      },
      {
        name: "contribute",
        discriminator: [2],
        accounts: [
          {
            name: "launch",
            pda: {
              seeds: [
                { kind: "const", value: [108, 97, 117, 110, 99, 104] },
                { kind: "account", path: "launch.admin" },
              ],
            },
          },
        ],
        args: [],
      },
    ],
  };

  const bytes = new TextEncoder().encode(JSON.stringify(source));
  const { programId } = projectIdlV1(bytes);
  const spec = parseIdlV1(bytes, programId).programSpec;

  expect(spec.pdas).not.toHaveProperty("launch");
  expect(spec.instructions[0]?.accounts[0]?.resolution.category).toBe("pdaInline");
  expect(spec.instructions[1]?.accounts[0]?.resolution.category).toBe("pdaInline");
});

test("reports a known but unexpected hash kind consistently", () => {
  const value = `arete:h1:program-spec:sha256:${"ab".repeat(32)}`;
  expect(() => parseHashId(value, "idl-content")).toThrowError(
    expect.objectContaining({ code: "unexpected-kind" }),
  );
});

test("validates public Program Read binding identifiers at runtime", () => {
  const value = `prb_${"Az0_-".repeat(6)}Az`;
  expect(parseProgramReadBindingId(value)).toBe(value);
  expect(isProgramReadBindingId(value)).toBe(true);
  expect(isProgramReadBindingId("prb_not-valid")).toBe(false);
  expect(() => parseProgramReadBindingId("prb_not-valid")).toThrowError(
    expect.objectContaining({ code: "invalid-projection" }),
  );
});

interface VectorCorpus {
  decoderFixtureErrorCategories: string[];
  errorCodes: string[];
  failureVectors: FailureVector[];
  hashIdVectors: HashIdVector[];
  hashVectors: HashVector[];
  idlFailureVectors: IdlVector[];
  idlVectors: IdlVector[];
  kinds: typeof IDENTITY_REGISTRY;
  nonHashIdentities: typeof NON_HASH_IDENTITY_REGISTRY;
  profiles: string[];
  protocol: { algorithm: string; label: string; version: number };
  projectionVectors: {
    compilerV1: {
      projection: {
        schema: string;
        sources: { path: string; bytesHex: string }[];
      };
      expectedHash: string;
    };
    sdkDefinitionV1: { projection: any; expectedHash: string };
  };
  releaseVectors: ReleaseVector[];
  decoderFixtureVectors: DecoderFixtureVector[];
  schema: string;
  selfHashVectors: SelfHashVector[];
}

interface EncodedBytes {
  encoding: "hex" | "utf8";
  data: string;
  explicitProgramId?: string | null;
}

interface FailureVector {
  id: string;
  operation: string;
  expectedError: string;
  input: any;
}

interface HashIdVector {
  id: string;
  input: string;
  valid: boolean;
  expectedKind?: string;
  expectedDigestHex?: string;
  expectedError?: string;
}

interface HashVector {
  id: string;
  kind: any;
  profile: any;
  input: any;
  expected: ExpectedHash;
}

interface ExpectedHash {
  canonicalPayloadHex: string;
  preimageHex: string;
  digestHex: string;
  hashId: string;
}

interface IdlVector {
  id: string;
  input: EncodedBytes;
  expected?: any;
  expectedError?: string;
}

interface ReleaseVector {
  id: string;
  projection: any;
  expected: ExpectedHash;
}

interface SelfHashVector {
  id: string;
  input: JsonValue;
  expectedProjection: JsonValue;
  expected: ExpectedHash;
}

interface DecoderFixtureVector {
  id: string;
  input: any;
  expectedProjection: JsonValue;
  expected: ExpectedHash;
}

const corpus = JSON.parse(
  readFileSync(
    new URL("../../../test-vectors/hash-v1.json", import.meta.url),
    "utf8",
  ),
) as VectorCorpus;

function inputBytes(input: EncodedBytes): Uint8Array {
  return input.encoding === "hex"
    ? hexToBytes(input.data)
    : new TextEncoder().encode(input.data);
}

function tupleFields(input: any): TupleField[] {
  return input.fields.map((field: any) => ({
    label: field.label,
    value:
      field.valueUtf8 === undefined
        ? hexToBytes(field.valueHex)
        : new TextEncoder().encode(field.valueUtf8),
  }));
}

function treeEntries(input: any): ArtifactTreeEntry[] {
  return input.entries.map((entry: any) =>
    entry.type === "symlink"
      ? { path: entry.path, type: "symlink" }
      : { path: entry.path, bytes: hexToBytes(entry.bytesHex), type: "file" },
  );
}

function expectHash(
  expected: ExpectedHash,
  payload: Uint8Array,
  id: string,
): void {
  const parsed = parseHashId(id);
  expect(bytesToHex(payload)).toBe(expected.canonicalPayloadHex);
  expect(
    bytesToHex(
      framedPreimage(
        parsed.kind,
        IDENTITY_REGISTRY.find((x) => x.kind === parsed.kind)!.profile,
        payload,
      ),
    ),
  ).toBe(expected.preimageHex);
  expect(parsed.digestHex).toBe(expected.digestHex);
  expect(id).toBe(expected.hashId);
}

describe("shared hash-v1 vectors", () => {
  test("registry and protocol are identical", () => {
    expect(corpus.schema).toBe("arete.hash-vectors/v1");
    expect(corpus.protocol).toMatchObject({
      algorithm: HASH_ALGORITHM,
      label: HASH_PROTOCOL_LABEL,
      version: HASH_PROTOCOL_VERSION,
    });
    expect(corpus.profiles).toEqual(CANONICALIZATION_PROFILES);
    expect(corpus.kinds).toEqual(IDENTITY_REGISTRY);
    expect(corpus.nonHashIdentities).toEqual(NON_HASH_IDENTITY_REGISTRY);
    expect(corpus.decoderFixtureErrorCategories).toEqual(
      DECODER_FIXTURE_ACCOUNT_DECODE_ERROR_CATEGORIES,
    );
    expect(
      corpus.kinds.find((item) => item.kind === "decoder-content")?.visibility,
    ).toBe("internal-only");
    expect(
      corpus.kinds.find((item) => item.kind === "decoder-fixture-set")?.visibility,
    ).toBe("internal-only");
    expect(
      corpus.kinds.find((item) => item.kind === "deployment-release")?.visibility,
    ).toBe("authenticated-owner");
    expectTypeOf<"compiler">().toMatchTypeOf<PublicHashKind>();
    expectTypeOf<"program-spec">().toMatchTypeOf<PublicHashKind>();
    expectTypeOf<AuthenticatedOwnerHashKind>().toEqualTypeOf<"deployment-release">();
    expectTypeOf<"decoder-content">().toMatchTypeOf<RawBytesHashKind>();
  });

  test.each(corpus.hashIdVectors)("HashId $id", (vector) => {
    if (vector.valid) {
      const parsed = parseHashId(vector.input);
      expect(parsed.kind).toBe(vector.expectedKind);
      expect(parsed.digestHex).toBe(vector.expectedDigestHex);
    } else {
      expect(() => parseHashId(vector.input)).toThrowError(
        expect.objectContaining({ code: vector.expectedError }),
      );
    }
  });

  test.each(corpus.hashVectors)("hash $id", (vector) => {
    let payload: Uint8Array;
    let id: string;
    if (vector.profile === "raw-bytes-v1") {
      payload = inputBytes(vector.input);
      id = hashRawBytes(vector.kind, payload);
    } else if (vector.profile === "arete-jcs-v1") {
      payload = canonicalizeJsonBytes(inputBytes(vector.input));
      id = hashJsonBytes(vector.kind, inputBytes(vector.input));
    } else if (vector.profile === "framed-tuple-v1") {
      const fields = tupleFields(vector.input);
      payload = framedTuplePayload(fields);
      id = hashFramedTuple(vector.kind, fields);
    } else {
      const entries = treeEntries(vector.input);
      payload = artifactTreePayload(entries);
      id = hashArtifactTree(vector.kind, entries);
    }
    expectHash(vector.expected, payload, id);
  });

  test.each(corpus.failureVectors)("failure $id", (vector) => {
    const operation = (): unknown => {
      if (vector.operation === "arete-jcs-v1") {
        return canonicalizeJsonBytes(inputBytes(vector.input));
      }
      if (vector.operation === "framed-tuple-v1")
        return framedTuplePayload(tupleFields(vector.input));
      if (vector.operation === "artifact-tree-v1")
        return artifactTreePayload(treeEntries(vector.input));
      if (vector.operation === "parse-profile") {
        return parseCanonicalizationProfile(vector.input.profile);
      }
      if (vector.operation === "remove-artifact-hash") {
        return projectWithoutArtifactHash(vector.input.value);
      }
      if (vector.operation === "execute-vector") {
        return hashRawBytes(vector.input.kind, inputBytes(vector.input.input));
      }
      if (vector.operation === "program-spec-v1") {
        const projection = structuredClone(
          vector.input.projection,
        ) as ProgramSpecV1;
        if (vector.input.normalizationVersionOverride !== undefined) {
          projection.idlSnapshot.normalizationVersion =
            vector.input.normalizationVersionOverride;
        }
        return hashProgramSpecV1(projection);
      }
      if (vector.operation === "oss-program-release-v1") {
        return hashOssGeneratedProgramReleaseV1(vector.input.projection);
      }
      if (vector.operation === "decoder-fixture-set-v2") {
        return validateDecoderFixtureSetV2(vector.input.projection);
      }
      if (vector.operation === "hosted-program-release-v2") {
        return validateHostedManagedProgramReleaseV2(vector.input.projection);
      }
      throw new Error(`unknown operation ${vector.operation}`);
    };
    expect(operation).toThrowError(
      expect.objectContaining({ code: vector.expectedError }),
    );
  });

  test.each(corpus.selfHashVectors)("self hash $id", (vector) => {
    const projection = projectWithoutArtifactHash(vector.input);
    expect(projection).toEqual(vector.expectedProjection);
    expectHash(
      vector.expected,
      canonicalizeJcs(projection),
      hashJcs("ast-portable", projection),
    );
  });

  test.each(corpus.idlVectors)("IDL $id", (vector) => {
    const document = parseIdlV1(
      inputBytes(vector.input),
      vector.input.explicitProgramId,
    );
    expect(document.programId).toBe(vector.expected.programId);
    expect(document.contentProjection).toEqual(
      vector.expected.contentProjection,
    );
    expect(document.portableProjection).toEqual(
      vector.expected.portableProjection,
    );
    expect(document.normalizedSnapshot).toEqual(
      vector.expected.normalizedSnapshot,
    );
    expect(document.programSpec).toEqual(vector.expected.programSpec);
    expect(document.ossRelease).toEqual(vector.expected.ossRelease);
    expect(document.hashes.source).toBe(vector.expected.source.hashId);
    expect(document.hashes.content).toBe(vector.expected.content.hashId);
    expect(document.hashes.portable).toBe(vector.expected.portable.hashId);
    expect(document.hashes.normalized).toBe(vector.expected.normalized.hashId);
    expect(document.hashes.programSpec).toBe(
      vector.expected.programSpecIdentity.hashId,
    );
    expect(document.hashes.ossRelease).toBe(
      vector.expected.ossReleaseIdentity.hashId,
    );
  });

  test.each(corpus.idlFailureVectors)("IDL failure $id", (vector) => {
    expect(() =>
      parseIdlV1(inputBytes(vector.input), vector.input.explicitProgramId),
    ).toThrowError(expect.objectContaining({ code: vector.expectedError }));
  });

  test.each(corpus.releaseVectors)("release $id", (vector) => {
    const id = vector.projection.releaseProfile === "oss-generated"
      ? hashOssGeneratedProgramReleaseV1(
          vector.projection as OssGeneratedProgramReleaseV1,
        )
      : vector.projection.schema === "arete.program-release/v2"
        ? hashHostedManagedProgramReleaseV2(vector.projection)
        : hashJcs("program-release", vector.projection);
    expectHash(vector.expected, canonicalizeJcs(vector.projection), id);
  });

  test("hosted Release V2 without upgrade authority matches shared bytes and hash", () => {
    const vector = corpus.releaseVectors.find(
      (item) =>
        item.id === "release-hosted-managed-v2-upgradeable-no-authority",
    )!;
    expect(
      vector.projection.executableIdentity.loader.upgradeAuthority,
    ).toEqual({ kind: "none" });
    const projection = validateHostedManagedProgramReleaseV2(vector.projection);
    expectHash(
      vector.expected,
      canonicalizeJcs(projection as unknown as JsonValue),
      hashHostedManagedProgramReleaseV2(projection),
    );
  });

  test.each(corpus.decoderFixtureVectors)("decoder fixture $id", (vector) => {
    const projection = validateDecoderFixtureSetV2(vector.input);
    expect(projection).toEqual(vector.expectedProjection);
    expectHash(
      vector.expected,
      canonicalizeJcs(projection as unknown as JsonValue),
      hashDecoderFixtureSetV2(vector.input),
    );
  });
});

describe("typed SDK identity projections", () => {
  test("compiler and SDK definition V1 are stable typed identities", () => {
    const compilerVector = corpus.projectionVectors.compilerV1;
    const sources = compilerVector.projection.sources.map((source) => ({
      path: source.path,
      bytes: hexToBytes(source.bytesHex),
    }));
    const first = createCompilerV1([...sources].reverse());
    const second = createCompilerV1(sources);
    const compilerHash = hashCompilerV1(first);
    expect(compilerHash).toBe(hashCompilerV1(second));
    expect(compilerHash).toBe(compilerVector.expectedHash);

    const definitionVector = corpus.projectionVectors.sdkDefinitionV1;
    const definition = createSdkDefinitionV1(
      definitionVector.projection.inputHash as ProgramSpecHash,
      compilerHash,
    );
    expect(definition).toEqual(definitionVector.projection);
    expect(hashSdkDefinitionV1(definition)).toBe(definitionVector.expectedHash);
  });
});

describe("strict JSON values", () => {
  test("object order is never identity", () => {
    expect(hashJcs("idl-content", { b: 1, a: { d: 4, c: 3 } })).toBe(
      hashJcs("idl-content", { a: { c: 3, d: 4 }, b: 1 }),
    );
  });

  test("rejects non-finite and unsafe object numbers", () => {
    expect(() =>
      canonicalizeJcs(Number.NaN as unknown as JsonValue),
    ).toThrowError(expect.objectContaining({ code: "non-finite-number" }));
    expect(() =>
      canonicalizeJcs(9_007_199_254_740_992 as JsonValue),
    ).toThrowError(expect.objectContaining({ code: "unsafe-json-integer" }));
  });

  test("release projections reject empty semantic identifiers", () => {
    const primary = corpus.idlVectors.find(
      (vector) => vector.id === "idl-primary",
    )!;
    const oss = {
      ...primary.expected.ossRelease,
      decoderEngineId: "",
    } as OssGeneratedProgramReleaseV1;
    expect(() => hashOssGeneratedProgramReleaseV1(oss)).toThrowError(
      expect.objectContaining({ code: "invalid-projection" }),
    );

    const hostedVector = corpus.releaseVectors.find(
      (vector) => vector.id === "release-hosted-managed-v2-upgradeable",
    )!;
    const hosted = {
      ...hostedVector.projection,
      decoderBindingId: "",
    } as HostedManagedProgramReleaseV2;
    expect(() => hashHostedManagedProgramReleaseV2(hosted)).toThrowError(
      expect.objectContaining({ code: "invalid-projection" }),
    );

    expect(() =>
      hashHostedManagedProgramReleaseV2({
        ...hostedVector.projection,
        decoderAbiVersion: "",
      } as HostedManagedProgramReleaseV2),
    ).toThrowError(expect.objectContaining({ code: "invalid-projection" }));
  });

  test("fixture V2 enforces case and byte bounds", () => {
    const baseline = structuredClone(corpus.decoderFixtureVectors[0]!.input);
    baseline.cases[0].accountDataHex = "00".repeat(
      DECODER_FIXTURE_MAX_ACCOUNT_BYTES + 1,
    );
    expect(() => validateDecoderFixtureSetV2(baseline)).toThrowError(
      expect.objectContaining({ code: "invalid-projection" }),
    );

    const tooMany = structuredClone(corpus.decoderFixtureVectors[0]!.input);
    const template = tooMany.cases[0];
    tooMany.cases = Array.from(
      { length: DECODER_FIXTURE_MAX_CASES + 1 },
      (_, index) => ({ ...template, id: `case-${index}` }),
    );
    expect(() => validateDecoderFixtureSetV2(tooMany)).toThrowError(
      expect.objectContaining({ code: "invalid-projection" }),
    );

    const tooLarge = structuredClone(corpus.decoderFixtureVectors[0]!.input);
    const fullCase = {
      ...tooLarge.cases[0],
      accountDataHex: "00".repeat(DECODER_FIXTURE_MAX_ACCOUNT_BYTES),
    };
    tooLarge.cases = Array.from(
      {
        length:
          DECODER_FIXTURE_MAX_TOTAL_ACCOUNT_BYTES /
            DECODER_FIXTURE_MAX_ACCOUNT_BYTES +
          1,
      },
      (_, index) => ({ ...fullCase, id: `total-${index}` }),
    );
    expect(() => validateDecoderFixtureSetV2(tooLarge)).toThrowError(
      expect.objectContaining({ code: "invalid-projection" }),
    );
  });
});
