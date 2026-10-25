/**
 * `arete-protocol`: check and build an Arete protocol.
 *
 *   arete-protocol check [entry] [--remote | --offline]
 *   arete-protocol build [entry] [--out <dir>] [--root <dir>] [--server-release]
 *   arete-protocol serve-handlers --review-key <id>=<pubkey> --ctx-url <url> [...]
 *
 * `entry` defaults to `protocol.ts` and must default-export
 * `defineProtocol(...)`.
 */

import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";

import { ProtocolContractError } from "./contracts/errors.js";
import { ProtocolDefinitionError } from "./define.js";
import { StackGenerationError } from "./stack/generate.js";
import {
  BUILD_FILES,
  ProtocolBuildError,
  buildProtocol,
  checkProtocol,
  releaseUploadBody,
  writeBuild,
} from "./build/build.js";
import { checkRemotely, remoteCheckTarget } from "./remote.js";
import {
  DEFAULT_PREPARE_TIMEOUT_MS,
  HANDLER_HOST_READY_SCHEMA,
  parseListenAddress,
  parseReviewKey,
  startHandlerHost,
  type HandlerHost,
  type HostLogLevel,
} from "./host/server.js";
import { deriveServerRelease } from "./build/artifacts.js";
import { PACKAGE_VERSION } from "./version.js";

const USAGE = `arete-protocol ${PACKAGE_VERSION}

Usage:
  arete-protocol check [entry] [--root <dir>] [--remote | --offline]
  arete-protocol build [entry] [--out <dir>] [--root <dir>] [--server-release]
  arete-protocol serve-handlers --review-key <keyId>=<pubkey> --ctx-url <url> [options]

entry   a module that default-exports defineProtocol(...) (default: protocol.ts)
--out   output directory for build (default: .arete/protocol)
--root  project root; artifact paths are relative to it (default: the entry's directory)
--server-release  also print the release hash a hosted service derives from the upload
--remote   require the server's check (fails without a secret key)
--offline  skip the server's check

check runs the local checks (syntax and names only), then, when ARETE_API_KEY
holds a secret key, the server's own upload checks (expression types and
templates against their program specs) without uploading anything.
ARETE_API_URL overrides the API (default https://api.arete.run).

To deploy, upload protocol-spec.json, plus handlers.mjs when a step has a
handler. The service derives the stack source and the release itself, so its
release hash differs from the local one (see --server-release).

Run \`arete-protocol serve-handlers --help\` for the local handler host.
`;

const SERVE_USAGE = `arete-protocol serve-handlers ${PACKAGE_VERSION}

Run reviewed handler bundles for Arete cloud dev environments over
the handler host protocol.

Usage:
  arete-protocol serve-handlers --review-key <keyId>=<base58 pubkey> --ctx-url <url>
                                [--listen 127.0.0.1:0] [--token-file <path>]
                                [--timeout-ms ${DEFAULT_PREPARE_TIMEOUT_MS}]

--review-key  a trusted release review key (repeatable)
--ctx-url     the context API's base URL; ctx calls go to <url>/handler-host/v1/ctx
--listen      loopback host:port to bind (default: 127.0.0.1:0, any free port)
--token-file  file holding the x-arete-handler-host-token secret
              (default: the ARETE_HANDLER_HOST_TOKEN environment variable)
--timeout-ms  per-invocation handler timeout (default: ${DEFAULT_PREPARE_TIMEOUT_MS})

When listening, prints one line to stdout:
  {"event":"ready","schema":"${HANDLER_HOST_READY_SCHEMA}","url":"http://127.0.0.1:<port>"}
Logs are JSON lines on stderr. SIGINT or SIGTERM shuts down gracefully.
`;

interface Parsed {
  command?: string;
  entry: string;
  out: string;
  root?: string;
  serverRelease: boolean;
  remote: boolean;
  offline: boolean;
}

function parse(argv: string[]): Parsed {
  const parsed: Parsed = {
    entry: "protocol.ts",
    out: ".arete/protocol",
    serverRelease: false,
    remote: false,
    offline: false,
  };
  const positional: string[] = [];
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]!;
    if (arg === "--out" || arg === "-o") parsed.out = argv[++i] ?? fail("--out needs a directory");
    else if (arg === "--root") parsed.root = argv[++i] ?? fail("--root needs a directory");
    else if (arg === "--server-release") parsed.serverRelease = true;
    else if (arg === "--remote") parsed.remote = true;
    else if (arg === "--offline") parsed.offline = true;
    else if (arg === "--help" || arg === "-h") parsed.command = "help";
    else if (arg.startsWith("-")) fail(`unknown option ${arg}`);
    else positional.push(arg);
  }
  parsed.command ??= positional[0];
  if (positional[1] !== undefined) parsed.entry = positional[1];
  if (parsed.remote && parsed.offline) fail("--remote and --offline cannot be combined");
  return parsed;
}

function fail(message: string, usage = USAGE): never {
  process.stderr.write(`error: ${message}\n\n${usage}`);
  process.exit(2);
}

function logLine(level: HostLogLevel, message: string, fields: Record<string, unknown> = {}): void {
  process.stderr.write(`${JSON.stringify({ ts: new Date().toISOString(), level, msg: message, ...fields })}\n`);
}

async function serveHandlers(argv: string[]): Promise<number> {
  const serveFail = (message: string): never => fail(message, SERVE_USAGE);
  let listen = "127.0.0.1:0";
  let tokenFile: string | undefined;
  let ctxUrl: string | undefined;
  let timeoutMs = DEFAULT_PREPARE_TIMEOUT_MS;
  const reviewKeys = new Map<string, Uint8Array>();
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]!;
    const value = () => argv[++i] ?? serveFail(`${arg} needs a value`);
    if (arg === "--help" || arg === "-h") {
      process.stdout.write(SERVE_USAGE);
      return 0;
    } else if (arg === "--listen") listen = value();
    else if (arg === "--token-file") tokenFile = value();
    else if (arg === "--ctx-url") ctxUrl = value();
    else if (arg === "--timeout-ms") {
      const text = value();
      timeoutMs = Number(text);
      if (!/^\d+$/.test(text) || timeoutMs < 1) serveFail(`--timeout-ms must be a positive integer, got '${text}'`);
    } else if (arg === "--review-key") {
      try {
        const [id, key] = parseReviewKey(value());
        if (reviewKeys.has(id)) serveFail(`--review-key '${id}' is given twice`);
        reviewKeys.set(id, key);
      } catch (error) {
        serveFail((error as Error).message);
      }
    } else serveFail(`unknown option ${arg}`);
  }
  if (ctxUrl === undefined) serveFail("--ctx-url is required");
  if (reviewKeys.size === 0) serveFail("at least one --review-key is required");
  let address: { host: string; port: number };
  try {
    address = parseListenAddress(listen);
  } catch (error) {
    return serveFail((error as Error).message);
  }
  let token: string;
  if (tokenFile !== undefined) {
    try {
      token = readFileSync(tokenFile, "utf8").replace(/\r?\n$/, "");
    } catch (error) {
      return serveFail(`cannot read --token-file: ${(error as Error).message}`);
    }
  } else {
    token = process.env.ARETE_HANDLER_HOST_TOKEN ?? "";
  }
  if (token.length === 0) serveFail("a token is required: pass --token-file or set ARETE_HANDLER_HOST_TOKEN");

  let host: HandlerHost;
  try {
    host = await startHandlerHost({
      token,
      reviewKeys,
      ctxUrl: ctxUrl!,
      host: address!.host,
      port: address!.port,
      timeoutMs,
      log: logLine,
    });
  } catch (error) {
    logLine("error", "the handler host failed to start", { error: (error as Error).message });
    return 1;
  }
  const running = host;
  // Install the signal handlers before announcing readiness, so a supervisor
  // that stops the host right after the ready line still gets a clean exit.
  const stopped = new Promise<number>((resolveExit) => {
    let stopping = false;
    const stop = (signal: string) => {
      if (stopping) return;
      stopping = true;
      logLine("info", "shutting down", { signal });
      running.close().then(
        () => resolveExit(0),
        () => resolveExit(1),
      );
    };
    process.on("SIGINT", () => stop("SIGINT"));
    process.on("SIGTERM", () => stop("SIGTERM"));
  });
  process.stdout.write(`${JSON.stringify({ event: "ready", schema: HANDLER_HOST_READY_SCHEMA, url: host.url })}\n`);
  logLine("info", "handler host listening", { url: host.url, reviewKeys: [...reviewKeys.keys()] });
  return stopped;
}

function describe(finding: Record<string, unknown>): string {
  if (typeof finding.file === "string") {
    return `${finding.file}:${finding.line}:${finding.column}: ${finding.message} [${finding.code}]`;
  }
  if (typeof finding.path === "string" && typeof finding.offset === "number") {
    return `${finding.path} @${finding.offset}+${finding.length}: ${finding.message} [${finding.code}]`;
  }
  return String(finding.message);
}

const SYNTAX_ONLY =
  "syntax only; run with credentials for the full check (set ARETE_API_KEY to a secret key, or run `a4 protocol deploy --dry-run`)";

async function check(args: Parsed, options: { entry: string; root?: string }): Promise<number> {
  const result = await checkProtocol(options);
  for (const warning of result.warnings) process.stderr.write(`warning: ${warning}\n`);
  const findings = [...result.expressionFindings, ...result.staticFindings];
  for (const finding of findings) process.stderr.write(`${describe(finding as never)}\n`);
  const label = `${result.lowered.spec.name}@${result.lowered.spec.version}`;
  const target = args.offline ? { reason: "--offline" } : remoteCheckTarget();
  if ("reason" in target) {
    if (args.remote) fail(`--remote needs a secret key: ${target.reason}`);
    if (findings.length > 0) return 1;
    process.stdout.write(`ok: ${label} (${SYNTAX_ONLY})\n`);
    return 0;
  }
  const outcome = await checkRemotely(await releaseUploadBody(options, result), target);
  if (outcome.kind === "unavailable") {
    if (args.remote) {
      process.stderr.write(`error: ${outcome.reason}\n`);
      return 1;
    }
    process.stderr.write(`warning: ${outcome.reason}; only the local checks ran\n`);
    if (findings.length > 0) return 1;
    process.stdout.write(`ok: ${label} (${SYNTAX_ONLY})\n`);
    return 0;
  }
  if (outcome.kind === "rejected") {
    process.stderr.write(`server: ${outcome.error}${outcome.code ? ` [${outcome.code}]` : ""}\n`);
    for (const error of outcome.errors) {
      const where = error.path ? `${error.path}: ` : "";
      process.stderr.write(`  ${where}${error.message}${error.code ? ` [${error.code}]` : ""}\n`);
    }
    return 1;
  }
  if (findings.length > 0) {
    process.stderr.write(
      "note: the server accepts this spec, but the local checker reported the findings above, and `arete-protocol build` refuses a spec with findings\n",
    );
    return 1;
  }
  if (outcome.templateCheck === "pending") {
    const missing = outcome.missingProgramSpecs.map((entry) => entry.alias ?? "?").join(", ");
    process.stderr.write(
      `warning: the server's template check is pending until it has the program specs of ${missing || "the pinned programs"}; activation waits for them\n`,
    );
  }
  process.stdout.write(`ok: ${label} (checked by the server; release ${outcome.releaseHash})\n`);
  return 0;
}

async function main(): Promise<number> {
  if (process.argv[2] === "serve-handlers") return serveHandlers(process.argv.slice(3));
  const args = parse(process.argv.slice(2));
  if (args.command === undefined || args.command === "help") {
    process.stdout.write(USAGE);
    return args.command === "help" ? 0 : 2;
  }
  const options = { entry: resolve(args.entry), ...(args.root ? { root: resolve(args.root) } : {}) };
  try {
    if (args.command === "check") return await check(args, options);
    if (args.command === "build") {
      const result = await buildProtocol(options);
      for (const warning of result.warnings) process.stderr.write(`warning: ${warning}\n`);
      writeBuild(result, args.out);
      const release = JSON.parse(new TextDecoder().decode(result.files.get("build-receipt.json")!)) as {
        outputs: Record<string, string | null>;
      };
      process.stdout.write(`built ${result.lowered.spec.name}@${result.lowered.spec.version} into ${args.out}\n`);
      for (const name of BUILD_FILES) if (result.files.has(name)) process.stdout.write(`  ${name}\n`);
      process.stdout.write(`release ${release.outputs.releaseHash}\n`);
      const bundle = result.files.get("handlers.mjs") ?? null;
      const upload = ["protocol-spec.json", ...(bundle === null ? [] : ["handlers.mjs"])];
      process.stdout.write(`upload ${upload.map((name) => join(args.out, name)).join(" ")}\n`);
      if (args.serverRelease) {
        const derived = deriveServerRelease(result.lowered.spec, bundle);
        process.stdout.write(`server release ${derived.releaseHash}\n`);
      }
      return 0;
    }
    fail(`unknown command '${args.command}'`);
  } catch (error) {
    if (error instanceof ProtocolBuildError) {
      process.stderr.write(`error: ${error.message}\n`);
      for (const finding of error.findings) process.stderr.write(`  ${describe(finding as never)}\n`);
      return 1;
    }
    if (error instanceof ProtocolContractError) {
      process.stderr.write(`error: ${error.toString()}\n`);
      return 1;
    }
    if (error instanceof ProtocolDefinitionError || error instanceof StackGenerationError) {
      process.stderr.write(`error: ${error.message}\n`);
      return 1;
    }
    throw error;
  }
}

main().then(
  (code) => process.exit(code),
  (error: unknown) => {
    process.stderr.write(`${error instanceof Error ? (error.stack ?? error.message) : String(error)}\n`);
    process.exit(1);
  },
);
