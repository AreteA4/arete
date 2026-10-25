/**
 * `arete-protocol check` and `build`: load a `defineProtocol` module, lower
 * and validate it, run the author checks, bundle handlers with pinned
 * esbuild into one deterministic ESM file, and write the release artifacts.
 *
 * Node only. Every path written into an artifact is relative to the
 * project root, and no artifact records wall time, so two clean builds of
 * the same project produce byte-identical output.
 */

import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import type * as Esbuild from "esbuild";
import { canonicalizeJcs, hashJcs } from "@usearete/hash";

import { builtinTemplateWarnings, checkSpecExpressions, type ExpressionFinding } from "../check/expressions.js";
import { checkHandlerSource, isNodeBuiltin, type StaticFinding } from "../check/static.js";
import { compareUtf8 } from "../contracts/shape.js";
import { hashHandlerBundle } from "../contracts/index.js";
import type { BuildDependencyV1, ProtocolSpecV1 } from "../contracts/types.js";
import { idlFileName, isDefinedProtocol, lowerProtocol, type LoweredProtocol, type ProtocolDefinition } from "../define.js";
import { generateStackSource, type GenerateStackSourceOptions } from "../stack/generate.js";
import { checkProgramLock, findManifestRoot } from "./lock.js";
import {
  buildAnatomy,
  buildMachineMap,
  buildReceipt,
  buildRelease,
  compilerHash,
  compilerIdentity,
} from "./artifacts.js";

/**
 * esbuild is a dependency of this package, pinned so every build bundles
 * with the same compiler. It is loaded lazily: only `arete-protocol check`
 * and `build` need it, so apps that use the contracts or the handler ABI
 * never load a bundler.
 */
let esbuildModule: typeof Esbuild | undefined;
async function loadEsbuild(): Promise<typeof Esbuild> {
  if (esbuildModule) return esbuildModule;
  try {
    esbuildModule = (await import("esbuild")) as typeof Esbuild;
  } catch {
    throw new ProtocolBuildError(
      "arete-protocol build could not load esbuild, a dependency of @usearete/protocol; reinstall the package",
    );
  }
  return esbuildModule;
}

export const BUILD_FILES = [
  "protocol-spec.json",
  "handlers.mjs",
  "protocol-stack-source.json",
  "protocol-release.json",
  "build-receipt.json",
  "build-sources.json",
  "machine-map.json",
  "anatomy.json",
] as const;

const HANDLER_ENTRY_NAME = "arete-protocol-handlers.js";

export interface ProtocolBuildOptions {
  /** The module that default-exports `defineProtocol(...)`. */
  entry: string;
  /** The project root; paths in artifacts are relative to it. Defaults to the entry's directory. */
  root?: string;
  /**
   * Where `@usearete/protocol` resolves when the project does not install
   * it (this package's own fixtures). Defaults to this package's entry.
   */
  protocolModule?: string;
}

export interface CheckResult {
  lowered: LoweredProtocol;
  expressionFindings: ExpressionFinding[];
  staticFindings: StaticFinding[];
  /** Project-relative paths of the source files the definition loads. */
  sources: string[];
  /** Notes that do not fail the check (e.g. no arete.lock to check programs against). */
  warnings: string[];
  /** The directory of the nearest arete.toml, where installed IDL paths start. */
  manifestRoot?: string;
}

export interface BuildResult extends CheckResult {
  /** Artifact file name → exact bytes. */
  files: Map<string, Uint8Array>;
}

export class ProtocolBuildError extends Error {
  readonly findings: Array<ExpressionFinding | StaticFinding | { message: string }>;

  constructor(message: string, findings: Array<ExpressionFinding | StaticFinding | { message: string }> = []) {
    super(message);
    this.name = "ProtocolBuildError";
    this.findings = findings;
  }
}

function defaultProtocolModule(): string {
  const here = dirname(fileURLToPath(import.meta.url));
  // Source layout (src/build/build.ts) or the bundled CLI (dist/cli.js).
  for (const candidate of [resolve(here, "../index.ts"), resolve(here, "./index.js")]) {
    if (existsSync(candidate)) return candidate;
  }
  return resolve(here, "./index.js");
}

function posix(path: string): string {
  return path.split(sep).join("/");
}

function sha256Hex(bytes: Uint8Array | string): string {
  return createHash("sha256").update(bytes).digest("hex");
}

/** Resolve `@usearete/protocol` normally, falling back to this package for its own fixtures. */
function protocolResolver(protocolModule: string): Esbuild.Plugin {
  return {
    name: "arete-protocol-self",
    setup(build) {
      build.onResolve({ filter: /^@usearete\/protocol$/ }, async (args) => {
        if (args.pluginData === "arete-protocol-self") return undefined;
        const resolved = await build.resolve(args.path, {
          kind: args.kind,
          resolveDir: args.resolveDir,
          importer: args.importer,
          pluginData: "arete-protocol-self",
        });
        if (resolved.errors.length === 0) return { path: resolved.path, sideEffects: false };
        return { path: protocolModule, sideEffects: false };
      });
    },
  };
}

/**
 * The esbuild warnings about a dynamic `import()` or `require()` it could not
 * bundle, by message id. Other warnings (a package.json listing `types` after
 * `import` and `require`, say) are not about loading code and do not fail
 * the build. A dynamic load esbuild leaves in the output without a warning
 * still fails: the bundle text is checked for `import(` and `require` below.
 */
const DYNAMIC_LOAD_WARNINGS: ReadonlySet<string> = new Set([
  "empty-glob",
  "unsupported-dynamic-import",
  "unsupported-require-call",
]);

/** Handlers may not import Node built-ins; the protocol host gives them none. */
const rejectNodeBuiltins: Esbuild.Plugin = {
  name: "arete-protocol-no-node-builtins",
  setup(build) {
    build.onResolve({ filter: /.*/ }, (args) => {
      if (args.kind === "entry-point" || !isNodeBuiltin(args.path)) return undefined;
      return {
        errors: [{ text: `'${args.path}' is a Node built-in; handlers reach data only through ctx` }],
      };
    });
  },
};

/**
 * Drop the `// <path>` line esbuild writes before each module's code. The
 * path is relative to the working directory, so it records where each
 * package lives (a symlinked `file:` package resolves to its real location,
 * which can climb out of the project and name the home directory). Without
 * these lines the bundle bytes depend only on the code, not on where the
 * project and its packages are checked out.
 */
function stripModulePathComments(text: string, inputs: readonly string[]): string {
  const comments = new Set(inputs.map((input) => `// ${input}`));
  return text
    .split("\n")
    .filter((line) => !comments.has(line))
    .join("\n");
}

function formatMessages(messages: Esbuild.Message[]): Array<{ message: string }> {
  return messages.map((message) => ({
    message: message.location
      ? `${message.location.file}:${message.location.line}:${message.location.column + 1}: ${message.text}`
      : message.text,
  }));
}

async function loadDefinition(
  entry: string,
  root: string,
  protocolModule: string,
): Promise<{ definition: ProtocolDefinition; inputs: string[] }> {
  let result: Esbuild.BuildResult & { metafile: Esbuild.Metafile; outputFiles: Esbuild.OutputFile[] };
  try {
    result = await (await loadEsbuild()).build({
      entryPoints: [entry],
      absWorkingDir: root,
      bundle: true,
      format: "esm",
      platform: "node",
      target: "node18",
      write: false,
      metafile: true,
      logLevel: "silent",
      outfile: join(root, "definition.mjs"),
      plugins: [protocolResolver(protocolModule)],
    });
  } catch (error) {
    const failure = error as { errors?: Esbuild.Message[] };
    throw new ProtocolBuildError("could not load the protocol definition", formatMessages(failure.errors ?? []));
  }
  const directory = mkdtempSync(join(tmpdir(), "arete-protocol-"));
  try {
    const file = join(directory, "definition.mjs");
    writeFileSync(file, result.outputFiles[0]!.contents);
    const module = (await import(pathToFileURL(file).href)) as { default?: unknown };
    if (!isDefinedProtocol(module.default)) {
      throw new ProtocolBuildError(`${posix(relative(root, entry))} must default-export defineProtocol(...)`);
    }
    return { definition: module.default, inputs: Object.keys(result.metafile.inputs) };
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

function projectSources(inputs: string[], root: string): string[] {
  return inputs
    .filter((input) => !input.startsWith("../") && !input.split("/").includes("node_modules"))
    .filter((input) => existsSync(join(root, input)))
    .sort(compareUtf8);
}

/** Lower, validate and run the author checks without writing anything. */
export async function checkProtocol(options: ProtocolBuildOptions): Promise<CheckResult> {
  const entry = resolve(options.entry);
  const root = resolve(options.root ?? dirname(entry));
  const { definition, inputs } = await loadDefinition(entry, root, options.protocolModule ?? defaultProtocolModule());
  const lowered = lowerProtocol(definition);
  const warnings: string[] = builtinTemplateWarnings(lowered.spec);
  const manifestRoot = findManifestRoot(root);
  if (Object.keys(lowered.installedPrograms).length > 0) {
    const problems = checkProgramLock(lowered.installedPrograms, manifestRoot, warnings);
    if (problems.length > 0) {
      throw new ProtocolBuildError("installed programs do not match arete.lock", problems);
    }
  }
  const sources = projectSources(inputs, root);
  const staticFindings = Object.keys(lowered.handlers).length === 0
    ? []
    : sources
        .filter((source) => /\.(?:[cm]?[jt]sx?)$/.test(source))
        .flatMap((source) => checkHandlerSource(readFileSync(join(root, source), "utf8"), source));
  return {
    lowered,
    expressionFindings: checkSpecExpressions(lowered.spec),
    staticFindings,
    sources,
    warnings,
    ...(manifestRoot !== undefined ? { manifestRoot } : {}),
  };
}

/**
 * What an upload (or the server's release check) takes for a checked
 * protocol: the spec, plus the handler bundle as standard base64 when a step
 * has a handler. The bundle is the one `buildProtocol` writes.
 */
export async function releaseUploadBody(
  options: ProtocolBuildOptions,
  checked: CheckResult,
): Promise<{ spec: ProtocolSpecV1; bundleBase64?: string }> {
  const spec = checked.lowered.spec;
  if (Object.keys(checked.lowered.handlers).length === 0) return { spec };
  const entry = resolve(options.entry);
  const root = resolve(options.root ?? dirname(entry));
  const bundle = await bundleHandlers(checked.lowered, entry, root, options.protocolModule ?? defaultProtocolModule());
  return { spec, bundleBase64: Buffer.from(bundle.bytes).toString("base64") };
}

async function bundleHandlers(
  lowered: LoweredProtocol,
  entry: string,
  root: string,
  protocolModule: string,
): Promise<{ bytes: Uint8Array; inputs: string[] }> {
  let specifier = posix(relative(root, entry));
  if (!specifier.startsWith(".")) specifier = `./${specifier}`;
  const lines = [
    `import definition from ${JSON.stringify(specifier)};`,
    "const handler = (step) => {",
    "  const h = definition.steps[step].handler;",
    "  return typeof h === \"function\" ? h : (input, ctx) => h.prepare(input, ctx);",
    "};",
  ];
  for (const [index, step] of Object.keys(lowered.handlerExports).sort(compareUtf8).entries()) {
    const local = `__protocol_handler_${index}`;
    lines.push(`const ${local} = handler(${JSON.stringify(step)});`);
    lines.push(`export { ${local} as ${JSON.stringify(lowered.handlerExports[step])} };`);
  }
  let result: Esbuild.BuildResult & { metafile: Esbuild.Metafile; outputFiles: Esbuild.OutputFile[] };
  try {
    result = await (await loadEsbuild()).build({
      stdin: { contents: `${lines.join("\n")}\n`, resolveDir: root, sourcefile: HANDLER_ENTRY_NAME, loader: "js" },
      absWorkingDir: root,
      bundle: true,
      format: "esm",
      platform: "neutral",
      target: "es2022",
      mainFields: ["module", "main"],
      minify: false,
      sourcemap: false,
      legalComments: "none",
      charset: "utf8",
      write: false,
      metafile: true,
      logLevel: "silent",
      outfile: join(root, "handlers.mjs"),
      // Resolve packages to their real location (as Node does), whether
      // they are installed copies or symlinks (`file:` and workspace
      // packages); the bundle does not record that location.
      preserveSymlinks: false,
      plugins: [rejectNodeBuiltins, protocolResolver(protocolModule)],
    });
  } catch (error) {
    const failure = error as { errors?: Esbuild.Message[] };
    throw new ProtocolBuildError("could not bundle the handlers", formatMessages(failure.errors ?? []));
  }
  const warnings = result.warnings.filter((warning) => DYNAMIC_LOAD_WARNINGS.has(warning.id));
  if (warnings.length > 0) throw new ProtocolBuildError("the handlers use dynamic imports", formatMessages(warnings));
  const output = Object.values(result.metafile.outputs)[0];
  const text = stripModulePathComments(
    new TextDecoder().decode(result.outputFiles[0]!.contents),
    [...Object.keys(result.metafile.inputs), HANDLER_ENTRY_NAME],
  );
  const bytes = new TextEncoder().encode(text);
  for (const machinePath of new Set([root, homedir(), dirname(protocolModule)])) {
    if (machinePath.length > 1 && text.includes(machinePath)) {
      throw new ProtocolBuildError(`the handler bundle contains a machine path (${machinePath})`);
    }
  }
  const bundleFindings = checkHandlerSource(text, "handlers.mjs");
  if (bundleFindings.length > 0) {
    throw new ProtocolBuildError("the handler bundle reaches outside ctx", bundleFindings);
  }
  if (/\bimport\s*\(/.test(text)) {
    throw new ProtocolBuildError("the handler bundle contains a dynamic import() that esbuild could not resolve");
  }
  // Only inputs that contribute bytes; fully tree-shaken modules are not dependencies.
  const inputs = Object.entries(output?.inputs ?? {})
    .filter(([, input]) => input.bytesInOutput > 0)
    .map(([path]) => path);
  return { bytes, inputs };
}

function lockfileDependencies(root: string, inputs: string[]): { lockfileSha256: string; dependencies: BuildDependencyV1[] } {
  const lockPath = join(root, "package-lock.json");
  const lockBytes = existsSync(lockPath) ? readFileSync(lockPath) : new Uint8Array();
  const lock = lockBytes.length > 0 ? (JSON.parse(new TextDecoder().decode(lockBytes)) as { packages?: Record<string, { version?: string; integrity?: string }> }) : {};
  // Package name → its directory relative to the root, from each bundled node_modules input.
  const names = new Map<string, string>();
  for (const input of inputs) {
    const parts = input.split("/");
    const at = parts.lastIndexOf("node_modules");
    if (at < 0 || parts[at + 1] === undefined) continue;
    const scoped = parts[at + 1]!.startsWith("@");
    const name = scoped ? `${parts[at + 1]}/${parts[at + 2]}` : parts[at + 1]!;
    if (!names.has(name)) names.set(name, parts.slice(0, at + (scoped ? 3 : 2)).join("/"));
  }
  const dependencies = [...names.entries()]
    .sort(([a], [b]) => compareUtf8(a, b))
    .map(([name, directory]) => {
      const entry = lock.packages?.[`node_modules/${name}`];
      let version = entry?.version;
      if (version === undefined) {
        const manifest = join(root, directory, "package.json");
        version = existsSync(manifest)
          ? ((JSON.parse(readFileSync(manifest, "utf8")) as { version?: string }).version ?? "")
          : "";
      }
      return { name, version, integrity: entry?.integrity ?? "" };
    });
  return { lockfileSha256: sha256Hex(lockBytes), dependencies };
}

/** `[path, sha256]` of each project source, in `sources` order. */
function sourceEntries(root: string, sources: string[], copied: ReadonlyMap<string, Uint8Array>): [string, string][] {
  return sources.map((source) => [source, sha256Hex(copied.get(source) ?? readFileSync(join(root, source)))]);
}

function sourceTreeSha256(entries: [string, string][]): string {
  return sha256Hex(canonicalizeJcs(entries));
}

/**
 * Local only (never uploaded): the schema of `build-sources.json`, which
 * lists the sources behind the receipt's `sourceTreeSha256` so
 * `a4 protocol deploy --no-build` can tell a stale build apart and name the
 * files that changed. Paths are relative to the project root.
 */
export const PROTOCOL_BUILD_SOURCES_SCHEMA = "arete.protocol-build-sources/v1";

const canonical = (value: unknown): Uint8Array => canonicalizeJcs(value as never);

/** Build every release artifact in memory. Throws `ProtocolBuildError` when a check fails. */
export async function buildProtocol(
  options: ProtocolBuildOptions & { stackSource?: GenerateStackSourceOptions },
): Promise<BuildResult> {
  const entry = resolve(options.entry);
  const root = resolve(options.root ?? dirname(entry));
  const protocolModule = options.protocolModule ?? defaultProtocolModule();
  const checked = await checkProtocol({ ...options, entry, root, protocolModule });
  const problems = [...checked.expressionFindings, ...checked.staticFindings];
  if (problems.length > 0) throw new ProtocolBuildError("the protocol failed its checks", problems);

  const { lowered } = checked;
  const spec: ProtocolSpecV1 = lowered.spec;
  const files = new Map<string, Uint8Array>();
  const specBytes = canonical(spec);
  files.set("protocol-spec.json", specBytes);

  let handlerBundleHash = null;
  let bundleInputs: string[] = [];
  if (Object.keys(lowered.handlers).length > 0) {
    const bundle = await bundleHandlers(lowered, entry, root, protocolModule);
    files.set("handlers.mjs", bundle.bytes);
    handlerBundleHash = hashHandlerBundle(bundle.bytes);
    bundleInputs = bundle.inputs;
  }

  const stackSource = generateStackSource(spec, options.stackSource ?? { programs: lowered.programSources });
  files.set("protocol-stack-source.json", canonical(stackSource));
  const sourcePaths = new Set(checked.sources);
  const copiedSources = new Map<string, Uint8Array>();
  // The stack source names installed IDLs as `./idl/<file>`, beside it.
  for (const [alias, program] of Object.entries(lowered.installedPrograms).sort(([a], [b]) => compareUtf8(a, b))) {
    const source = program.programInstall.source;
    if (source.kind !== "idl") continue;
    const path = resolve(checked.manifestRoot ?? root, source.idl);
    if (!existsSync(path)) {
      throw new ProtocolBuildError(
        `program '${alias}' was installed from ${source.idl}, which is missing; restore it or reinstall the program`,
      );
    }
    const bytes = readFileSync(path);
    const relativePath = posix(relative(root, path));
    files.set(`idl/${idlFileName(source.idl)}`, bytes);
    sourcePaths.add(relativePath);
    copiedSources.set(relativePath, bytes);
  }

  const identity = compilerIdentity((await loadEsbuild()).version);
  const specHash = hashJcs("protocol-spec", spec as never);
  const stackSourceHash = hashJcs("protocol-stack-source", stackSource as never);
  const release = buildRelease(spec, {
    specHash,
    handlerBundleHash,
    stackSourceHash,
    compilerHash: compilerHash(identity),
  });
  files.set("protocol-release.json", canonical(release));
  const releaseHash = hashJcs("protocol-release", release as never);

  const { lockfileSha256, dependencies } = lockfileDependencies(root, bundleInputs);
  const recordedSources = [...sourcePaths].sort(compareUtf8);
  const sources = sourceEntries(root, recordedSources, copiedSources);
  const treeSha256 = sourceTreeSha256(sources);
  const receipt = buildReceipt({
    sourceTreeSha256: treeSha256,
    lockfileSha256,
    dependencies,
    compiler: identity,
    outputs: { specHash, handlerBundleHash, stackSourceHash, releaseHash },
  });
  files.set("build-receipt.json", canonical(receipt));
  files.set(
    "build-sources.json",
    canonical({
      schema: PROTOCOL_BUILD_SOURCES_SCHEMA,
      sourceTreeSha256: treeSha256,
      files: sources.map(([path, sha256]) => ({ path, sha256 })),
    }),
  );
  files.set("machine-map.json", canonical(buildMachineMap(spec)));
  files.set("anatomy.json", canonical(buildAnatomy(spec)));
  return { ...checked, sources: recordedSources, files };
}

/**
 * Write a build's files to `outDir`, removing a stale `handlers.mjs` and
 * replacing `idl/` (the installed IDLs the stack source reads).
 */
export function writeBuild(result: BuildResult, outDir: string): void {
  const out = isAbsolute(outDir) ? outDir : resolve(outDir);
  mkdirSync(out, { recursive: true });
  for (const name of BUILD_FILES) {
    const bytes = result.files.get(name);
    const path = join(out, name);
    if (bytes === undefined) rmSync(path, { force: true });
    else writeFileSync(path, bytes);
  }
  rmSync(join(out, "idl"), { recursive: true, force: true });
  for (const [name, bytes] of result.files) {
    if (!name.startsWith("idl/")) continue;
    mkdirSync(join(out, "idl"), { recursive: true });
    writeFileSync(join(out, name), bytes);
  }
}
