import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { canonicalizeJcs } from "@usearete/hash";
import { afterAll, describe, expect, it } from "vitest";

import {
  PROTOCOL_HANDLER_ABI_V1,
  parseContractJson,
  validateContract,
  type PrepareInputV1,
  type ProtocolHandlerFnV1,
} from "../index.js";
import { runHandler, type RecordedCtxCallV1 } from "../testing.js";
import { deriveServerRelease, SERVER_COMPILER_NAME } from "./artifacts.js";
import { BUILD_FILES, ProtocolBuildError, buildProtocol, checkProtocol, writeBuild } from "./build.js";

const here = dirname(fileURLToPath(import.meta.url));
const entry = resolve(here, "../../fixtures/token-raise/protocol.ts");
const decoder = new TextDecoder();
const temporary: string[] = [];

afterAll(() => {
  for (const directory of temporary) rmSync(directory, { recursive: true, force: true });
});

function outDir(): string {
  const directory = mkdtempSync(join(tmpdir(), "arete-protocol-build-"));
  temporary.push(directory);
  return directory;
}

function readTree(directory: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const name of readdirSync(directory).sort()) out[name] = readFileSync(join(directory, name), "hex");
  return out;
}

describe("arete-protocol build", () => {
  it("produces byte-identical artifacts from two clean builds", async () => {
    const first = outDir();
    const second = outDir();
    writeBuild(await buildProtocol({ entry }), first);
    writeBuild(await buildProtocol({ entry }), second);
    expect(readTree(first)).toEqual(readTree(second));
    expect(Object.keys(readTree(first))).toEqual([...BUILD_FILES].sort());
  });

  it("writes canonical, valid artifacts with no machine paths", async () => {
    const result = await buildProtocol({ entry });
    const json = (name: string) => parseContractJson(result.files.get(name)!);
    for (const [name, bytes] of result.files) {
      const text = decoder.decode(bytes);
      expect(text.includes(homedir())).toBe(false);
      expect(text.includes(here)).toBe(false);
      if (name.endsWith(".json")) expect(text).toBe(decoder.decode(canonicalizeJcs(JSON.parse(text))));
    }

    const fixture = parseContractJson(readFileSync(resolve(here, "../../../../test-vectors/protocol/token-raise.protocol-spec.json")));
    expect(decoder.decode(result.files.get("protocol-spec.json")!)).toBe(decoder.decode(canonicalizeJcs(fixture as never)));

    const specHash = validateContract("protocol-spec", json("protocol-spec.json"));
    const stackSourceHash = validateContract("protocol-stack-source", json("protocol-stack-source.json"));
    const releaseHash = validateContract("protocol-release", json("protocol-release.json"));
    expect(validateContract("protocol-build-receipt", json("build-receipt.json"))).toBeNull();

    const release = json("protocol-release.json") as Record<string, any>;
    expect(release.specHash).toBe(specHash);
    expect(release.stackSourceHash).toBe(stackSourceHash);
    expect(release.handlerBundleHash).toMatch(/^arete:h1:protocol-handler-bundle:sha256:/);
    expect(release.compilerHash).toMatch(/^arete:h1:compiler:sha256:/);
    expect(release.dependencies).toEqual({
      programs: [
        {
          alias: "meteoraPresale",
          programId: "presSVxnf9UU8jMxhgSMqaRwNiT36qeBdNeTRKjTdbj",
          programSpecHash: "arete:h1:program-spec:sha256:a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1",
        },
        {
          alias: "squadsV4",
          programId: "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf",
          programSpecHash: "arete:h1:program-spec:sha256:b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2",
        },
      ],
      stacks: [
        {
          alias: "meteoraPresale",
          stackManifestHash: "arete:h1:stack-manifest:sha256:c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3",
        },
      ],
      sdkExtensions: [],
    });

    const receipt = json("build-receipt.json") as Record<string, any>;
    expect(receipt.outputs).toEqual({
      specHash,
      handlerBundleHash: release.handlerBundleHash,
      stackSourceHash,
      releaseHash,
    });
    expect(receipt.lockfileSha256).toBe("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    expect(result.sources).toEqual(["handlers/createLaunch.ts", "protocol.ts"]);
    // build-sources.json names the files behind sourceTreeSha256, so a stale
    // build can be told apart without rebuilding.
    const sources = json("build-sources.json") as Record<string, any>;
    expect(sources.schema).toBe("arete.protocol-build-sources/v1");
    expect(sources.sourceTreeSha256).toBe(receipt.sourceTreeSha256);
    expect(sources.files.map((file: any) => file.path)).toEqual(result.sources);
    const tree = sources.files.map((file: any) => [file.path, file.sha256]);
    const { createHash } = await import("node:crypto");
    expect(createHash("sha256").update(canonicalizeJcs(tree)).digest("hex")).toBe(receipt.sourceTreeSha256);

    const map = json("machine-map.json") as Record<string, any>;
    expect(map.machines.instance.edges).toContainEqual({ from: "succeeded", to: "awaiting_approval", via: { step: "finalize" } });
    expect(map.machines.instance.edges).toContainEqual({ from: "open", to: "closed", via: { derive: 0 } });
    const anatomy = json("anatomy.json") as Record<string, any>;
    expect(anatomy.steps.contribute.instructions.map((slot: any) => `${slot.role}:${slot.kind}`)).toEqual([
      "runtime:computeBudget",
      "glue:createAta",
      "glue:wrapSol",
      "runtime:fee",
      "program:template",
      "runtime:memo",
    ]);
  });

  it("bundles the same bytes wherever the project and its packages live", async () => {
    // A stand-in `@usearete/protocol` package re-exporting this package's
    // source. One project installs a copy of it; the other links it from a
    // directory outside the project, as `npm install` does for `file:`.
    const sourceIndex = resolve(here, "../index.ts");
    const pkg = join(outDir(), "protocol-package");
    mkdirSync(pkg, { recursive: true });
    writeFileSync(
      join(pkg, "package.json"),
      JSON.stringify({ name: "@usearete/protocol", version: "0.0.0", type: "module", main: "./index.ts" }),
    );
    writeFileSync(join(pkg, "index.ts"), `export * from ${JSON.stringify(sourceIndex)};\n`);
    const project = (link: boolean): string => {
      const root = join(outDir(), link ? "linked" : "copied");
      cpSync(dirname(entry), root, { recursive: true });
      const scope = join(root, "node_modules", "@usearete");
      mkdirSync(scope, { recursive: true });
      if (link) symlinkSync(pkg, join(scope, "protocol"), "dir");
      else cpSync(pkg, join(scope, "protocol"), { recursive: true });
      return join(root, "protocol.ts");
    };
    // `protocolModule` points nowhere, so the bundle must use the installed package.
    const missing = join(outDir(), "missing", "index.ts");
    const copied = await buildProtocol({ entry: project(false), protocolModule: missing });
    const linked = await buildProtocol({ entry: project(true), protocolModule: missing });
    const copiedBundle = copied.files.get("handlers.mjs")!;
    expect(decoder.decode(linked.files.get("handlers.mjs")!)).toBe(decoder.decode(copiedBundle));
    expect(linked.files.get("protocol-release.json")).toEqual(copied.files.get("protocol-release.json"));
    const text = decoder.decode(copiedBundle);
    for (const path of [homedir(), tmpdir(), here, "node_modules", "protocol-package"]) {
      expect(text.includes(path)).toBe(false);
    }
    expect(text).not.toMatch(/^\/\/ /m);
  });

  it("predicts the server-derived release from the upload set", async () => {
    const result = await buildProtocol({ entry });
    const spec = parseContractJson(result.files.get("protocol-spec.json")!) as never;
    const bundle = result.files.get("handlers.mjs")!;
    const derived = deriveServerRelease(spec, bundle);
    const local = parseContractJson(result.files.get("protocol-release.json")!) as Record<string, unknown>;
    expect(validateContract("protocol-release", derived.release)).toBe(derived.releaseHash);
    expect(derived.release.specHash).toBe(local.specHash);
    expect(derived.release.handlerBundleHash).toBe(local.handlerBundleHash);
    expect(derived.release.dependencies).toEqual(local.dependencies);
    expect(derived.release.compilerHash).not.toBe(local.compilerHash);
    expect(derived.stackSource.generator.name).toBe(SERVER_COMPILER_NAME);
    expect(derived.receipt.compiler.name).toBe(SERVER_COMPILER_NAME);
    expect(() => deriveServerRelease(spec, null)).toThrow(/no handler bundle/);
  });

  it("bundles a handler that replays a recorded snapshot", async () => {
    const result = await buildProtocol({ entry });
    const directory = outDir();
    const file = join(directory, "handlers.mjs");
    writeFileSync(file, result.files.get("handlers.mjs")!);
    const bundle = (await import(pathToFileURL(file).href)) as Record<string, ProtocolHandlerFnV1>;
    expect(Object.keys(bundle)).toEqual(["createLaunch"]);

    const quoteMint = "So11111111111111111111111111111111111111112";
    const input: PrepareInputV1 = {
      abi: PROTOCOL_HANDLER_ABI_V1,
      step: "create",
      args: {
        deadline: "1790000000",
        softCap: "1000",
        hardCap: "5000",
        walletCap: "500",
        quoteMint,
      },
      resources: { launchDefaults: { tokenDecimals: 6 } },
      params: {},
      instance: null,
      participant: null,
      reads: {},
      wallet: "presSVxnf9UU8jMxhgSMqaRwNiT36qeBdNeTRKjTdbj",
      executionId: "00000000-0000-4000-8000-000000000001",
      ephemeral: {
        mint: "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr",
        presaleBase: "ComputeBudget111111111111111111111111111111",
      },
      programs: {
        meteoraPresale: "presSVxnf9UU8jMxhgSMqaRwNiT36qeBdNeTRKjTdbj",
        system: "11111111111111111111111111111111",
        splToken: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
      },
      fee: null,
      now: 1789000000,
      deployment: {
        id: "00000000-0000-4000-8000-0000000000d1",
        attestation: "Vote111111111111111111111111111111111111111",
        squadsProposer: "Stake11111111111111111111111111111111111111",
      },
    };
    const calls: RecordedCtxCallV1[] = [
      {
        method: "solana.account",
        args: { pubkey: quoteMint },
        result: {
          address: quoteMint,
          exists: true,
          slot: "300000000",
          lamports: "1461600",
          owner: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
          dataBase64: "AAAA",
        },
      },
    ];
    const operation = await runHandler(bundle.createLaunch!, input, calls);
    expect(operation.transactions).toHaveLength(1);
    expect(operation.transactions[0]!.instructions.map((ix) => ix.programId)).toEqual([
      "11111111111111111111111111111111",
      "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
      "presSVxnf9UU8jMxhgSMqaRwNiT36qeBdNeTRKjTdbj",
    ]);

    await expect(runHandler(bundle.createLaunch!, input, [])).rejects.toMatchObject({ code: "ctx-replay-divergence" });
    await expect(
      runHandler(bundle.createLaunch!, input, [{ ...calls[0]!, args: { pubkey: "11111111111111111111111111111111" } }]),
    ).rejects.toMatchObject({ code: "ctx-replay-divergence" });
    await expect(runHandler(bundle.createLaunch!, input, [...calls, calls[0]!])).rejects.toMatchObject({
      code: "ctx-replay-unconsumed",
    });
  });

  it("bundles a handler that imports a generated program module", async () => {
    // `programs/vote/` is the CLI's golden program module, importing its
    // runtime from `@usearete/sdk/program`, plus a curated extension shaped
    // like a registry program's (SPL addresses, PDA and ATA helpers,
    // prepared instructions, transactions and flows). The project gets a
    // stand-in `@usearete/sdk` that exports only `./program`, re-exporting
    // the SDK source, and the `zod` the SDK itself installs: an extension
    // that reached for the root entry would fail to resolve here, as the
    // root entry fails the handler bundle in a real project.
    const sdkSource = resolve(here, "../../../core");
    const sdkModules = join(sdkSource, "node_modules");
    if (!existsSync(join(sdkModules, "zod")) || !existsSync(join(sdkModules, "@noble", "ed25519"))) {
      throw new Error(`this test bundles the SDK source; run \`npm ci\` in ${sdkSource} first`);
    }
    const root = join(outDir(), "installed-program-handler");
    cpSync(resolve(here, "../../fixtures/installed-program-handler"), root, { recursive: true });
    const sdk = join(root, "node_modules", "@usearete", "sdk");
    mkdirSync(sdk, { recursive: true });
    writeFileSync(
      join(sdk, "package.json"),
      JSON.stringify({ name: "@usearete/sdk", version: "0.0.0", type: "module", exports: { "./program": "./program.ts" } }),
    );
    writeFileSync(join(sdk, "program.ts"), `export * from ${JSON.stringify(join(sdkSource, "src", "program.ts"))};\n`);
    symlinkSync(join(sdkModules, "zod"), join(root, "node_modules", "zod"), "dir");

    const result = await buildProtocol({ entry: join(root, "protocol.ts"), protocolModule: resolve(here, "../index.ts") });
    const text = decoder.decode(result.files.get("handlers.mjs")!);
    // The generated module and the SDK program runtime are in the bundle.
    expect(text).toContain("@usearete/sdk/program-runtime-extensions");
    expect(text).toContain("Vote111111111111111111111111111111111111111");
    for (const host of ["WebSocket", "XMLHttpRequest", "Buffer.from", "process.env"]) expect(text).not.toContain(host);

    const file = join(outDir(), "handlers.mjs");
    writeFileSync(file, result.files.get("handlers.mjs")!);
    const bundle = (await import(pathToFileURL(file).href)) as Record<string, ProtocolHandlerFnV1>;
    const wallet = "Stake11111111111111111111111111111111111111";
    const operation = await runHandler(
      bundle.ping!,
      {
        abi: PROTOCOL_HANDLER_ABI_V1,
        step: "open",
        args: {},
        resources: {},
        params: {},
        instance: null,
        participant: null,
        reads: {},
        wallet,
        executionId: "00000000-0000-4000-8000-000000000002",
        ephemeral: {},
        programs: { vote: "Vote111111111111111111111111111111111111111", system: "11111111111111111111111111111111" },
        fee: null,
        now: 1789000000,
        deployment: { id: "00000000-0000-4000-8000-0000000000d2", attestation: wallet, squadsProposer: wallet },
      },
      [],
    );
    expect(operation.transactions[0]!.instructions).toEqual([
      {
        programId: "Vote111111111111111111111111111111111111111",
        keys: [{ pubkey: wallet, isSigner: true, isWritable: true }],
        dataBase64: "AQIDBAUGBwg=",
      },
    ]);
  });

  /** A one-step protocol whose handler runs `body`, after `imports`. */
  function handlerProject(imports: string[], body: string): string {
    const directory = outDir();
    writeFileSync(
      join(directory, "protocol.ts"),
      [
        ...imports,
        'import { defineProtocol } from "@usearete/protocol";',
        "export default defineProtocol({",
        '  name: "probe", version: "0.1.0", instanceVisibility: "shared",',
        '  programs: {',
        '    system: { builtin: "system" },',
        '    probeProgram: {',
        '      programId: "Stake11111111111111111111111111111111111111",',
        '      programSpecHash: "arete:h1:program-spec:sha256:d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4",',
        "    },",
        "  },",
        '  machines: { instance: { initial: "open", states: { open: { terminal: true } } } },',
        "  steps: { open: { scope: 'create', authority: 'creator', programs: ['system'],",
        "    bind: { accounts: { state: 'wallet' } },",
        `    handler: async () => { ${body} return { abi: 'arete.protocol-handler/v1', transactions: [] }; } } },`,
        '  stack: { instanceAccounts: { state: { program: "probeProgram", account: "State" } }, activity: [] },',
        "});",
      ].join("\n"),
    );
    return join(directory, "protocol.ts");
  }

  it.each(["handler", "definition", "default", "class", "prepare"])("bundles a valid handler export named %s", async (name) => {
    const file = handlerProject([], "");
    writeFileSync(file, readFileSync(file, "utf8").replace("handler: async", `export: ${JSON.stringify(name)}, handler: async`));
    expect((await checkProtocol({entry:file})).lowered.handlerExports.open).toBe(name);
    const result = await buildProtocol({entry:file});
    const directory = outDir();
    writeBuild(result,directory);
    const module = await import(/* @vite-ignore */ pathToFileURL(join(directory,"handlers.mjs")).href);
    expect(typeof module[name]).toBe("function");
  });

  it("records copied local IDLs in source receipts", async () => {
    const directory = outDir();
    cpSync(resolve(here,"../../fixtures/program-imports"),directory,{recursive:true});
    const file = join(directory,"protocol.ts");
    const first = await buildProtocol({entry:file});
    const idl = join(directory,"idl/tip_jar.json");
    writeFileSync(idl, readFileSync(idl,"utf8") + "\n");
    const second = await buildProtocol({entry:file});
    const json = (result: typeof first, name: string) => JSON.parse(decoder.decode(result.files.get(name)!));
    const sources = json(second,"build-sources.json");
    expect(second.sources).toContain("idl/tip_jar.json");
    expect(sources.files).toContainEqual(expect.objectContaining({path:"idl/tip_jar.json"}));
    expect(sources.sourceTreeSha256).not.toBe(json(first,"build-sources.json").sourceTreeSha256);
    expect(json(second,"build-receipt.json").sourceTreeSha256).toBe(sources.sourceTreeSha256);
    expect(Array.from([...second.files.entries()].find(([path])=>path.startsWith("idl/"))![1])).toEqual(Array.from(readFileSync(idl)));
  });

  /**
   * Link a package from outside the project into its node_modules, as a
   * `file:` dependency is: esbuild reports no warnings for files whose real
   * path is under node_modules. Its exports list `types` last.
   */
  function linkPackage(entry: string, name: string, code: string): void {
    const pkg = join(outDir(), name);
    mkdirSync(pkg, { recursive: true });
    writeFileSync(
      join(pkg, "package.json"),
      JSON.stringify({
        name,
        version: "1.0.0",
        type: "module",
        exports: { ".": { import: "./index.js", require: "./index.js", types: "./index.d.ts" } },
      }),
    );
    writeFileSync(join(pkg, "index.js"), code);
    mkdirSync(join(dirname(entry), "node_modules"), { recursive: true });
    symlinkSync(pkg, join(dirname(entry), "node_modules", name), "dir");
  }

  it("bundles a package whose exports list types after import and require", async () => {
    // esbuild warns about the unreachable `types` condition, and the warning
    // names "require"; it is not a dynamic load.
    const entry = handlerProject(['import { memo } from "late-types";'], "memo(1);");
    linkPackage(entry, "late-types", "export const memo = (n) => n + 1;\n");
    const result = await buildProtocol({ entry, protocolModule: resolve(here, "../index.ts") });
    expect(decoder.decode(result.files.get("handlers.mjs")!)).toContain("n + 1");
  });

  it("rejects a dynamic require esbuild could not resolve", async () => {
    // In a dependency, so the project's own source checks do not see it first.
    const entry = handlerProject(['import { load } from "loader";'], "load('x');");
    linkPackage(entry, "loader", "export const load = (name) => require('./missing-' + name);\n");
    const failure = await buildProtocol({ entry, protocolModule: resolve(here, "../index.ts") }).catch(
      (error: unknown) => error,
    );
    expect(failure).toBeInstanceOf(ProtocolBuildError);
    expect((failure as Error).message).toMatch(/dynamic imports/);
  });

  it("rejects a dynamic import() left in the bundle", async () => {
    const entry = handlerProject([], "const name = String(Date.now()); await import(name);");
    const failure = await buildProtocol({ entry, protocolModule: resolve(here, "../index.ts") }).catch(
      (error: unknown) => error,
    );
    expect(failure).toBeInstanceOf(ProtocolBuildError);
  });

  it("rejects a handler that imports a Node built-in", async () => {
    const directory = outDir();
    writeFileSync(
      join(directory, "protocol.ts"),
      [
        'import { readFileSync } from "node:fs";',
        'import { defineProtocol } from "@usearete/protocol";',
        "export default defineProtocol({",
        '  name: "bad", version: "0.1.0", instanceVisibility: "shared",',
        '  programs: { system: { builtin: "system" } },',
        '  machines: { instance: { initial: "open", states: { open: { terminal: true } } } },',
        "  steps: { open: { scope: 'create', authority: 'creator', programs: ['system'],",
        "    handler: async () => { readFileSync('x'); return { abi: 'arete.protocol-handler/v1', transactions: [] }; } } },",
        "  stack: { activity: [] },",
        "});",
      ].join("\n"),
    );
    const failure = await buildProtocol({ entry: join(directory, "protocol.ts"), protocolModule: resolve(here, "../index.ts") }).catch(
      (error: unknown) => error,
    );
    expect(failure).toBeInstanceOf(ProtocolBuildError);
    expect((failure as ProtocolBuildError).findings).toContainEqual(
      expect.objectContaining({ code: "forbidden-import", file: "protocol.ts", line: 1 }),
    );
  });
});
