import { describe, expect, it } from "vitest";

import {
  createLiveSpecArtifact,
  createStackManifestArtifact,
  loadLiveSpec,
  validateLiveSpecArtifact,
} from "./artifacts.js";
import { hashJcs } from "./canonical.js";
import type { JsonValue } from "./types.js";

const emptyLive = () =>
  createLiveSpecArtifact({
    schema: "arete.live-spec/v1",
    compilerContractVersion: "arete-live-compiler/v1",
    wireContractVersion: "arete-live-wire/v1",
    programs: [],
    entities: [],
  });

describe("public artifacts", () => {
  it("creates deterministic typed live and stack identities", () => {
    const live = emptyLive();
    expect(emptyLive().artifactHash).toBe(live.artifactHash);
    const manifest = createStackManifestArtifact({
      schema: "arete.stack-manifest/v1",
      name: "EmptyStack",
      programs: [],
      liveSpecs: [{ artifactHash: live.artifactHash }],
      selectedViews: [],
    });
    expect(manifest.artifactHash).toMatch(/^arete:h1:stack-manifest:sha256:/);
  });

  it("preserves exact bytes and rejects hash mutation", () => {
    const live = emptyLive();
    const bytes = new TextEncoder().encode(JSON.stringify(live, null, 2));
    const loaded = loadLiveSpec(bytes);
    expect(loaded.originalBytes).toEqual(bytes);

    expect(() =>
      validateLiveSpecArtifact({
        ...live,
        artifactHash: live.artifactHash.replace(/.$/, "0") as typeof live.artifactHash,
      }),
    ).toThrow(/hash mismatch/);
  });

  it("rejects future majors and private decoder fields", () => {
    const live = emptyLive();
    expect(() => validateLiveSpecArtifact({ ...live, artifactVersion: "2.0.0" })).toThrow(
      /unsupported/,
    );
    expect(() =>
      createLiveSpecArtifact({
        ...live.payload,
        entities: [{ decoderBindingId: "private" }],
      }),
    ).toThrow(/private field/);
  });

  it("loads typed V2 live and multi-live manifest artifacts", async () => {
    const livePayload = {
      schema: "arete.live-spec/v2",
      compilerContractVersion: "arete-live-compiler/v2",
      wireContractVersion: "arete-live-wire/v2",
      programs: [],
      entities: [],
      programAdapters: [],
    } as const;
    const liveProjection = {
      artifactVersion: "1.0.0",
      kind: "live-spec",
      payload: livePayload,
    } as unknown as JsonValue;
    const live = { ...liveProjection, artifactHash: hashJcs("live-spec", liveProjection) };
    expect(loadLiveSpec(new TextEncoder().encode(JSON.stringify(live))).artifact.payload.schema)
      .toBe("arete.live-spec/v2");

    const manifestPayload = {
      schema: "arete.stack-manifest/v2",
      name: "Composed",
      programs: [],
      liveSpecs: [
        { alias: "first", artifactHash: live.artifactHash },
        { alias: "second", artifactHash: live.artifactHash },
      ],
      selectedViews: [],
    } as const;
    const manifestProjection = {
      artifactVersion: "1.0.0",
      kind: "stack-manifest",
      payload: manifestPayload,
    } as unknown as JsonValue;
    const manifest = {
      ...manifestProjection,
      artifactHash: hashJcs("stack-manifest", manifestProjection),
    };
    const { loadStackManifest } = await import("./artifacts.js");
    expect(loadStackManifest(new TextEncoder().encode(JSON.stringify(manifest))).artifact.payload.schema)
      .toBe("arete.stack-manifest/v2");
  });
});
