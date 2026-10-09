import type { Experimental_SandboxSession } from '@ai-sdk/provider-utils';
import type { Sandbox } from '@vercel/sandbox';
import type { HarnessSandboxTemplate } from '@ai-sdk/harness/agent';
import { createVercelNetworkSandboxSession } from '@ai-sdk/sandbox-vercel';
import { vercelCredentials } from './env.js';
import type { CommandResult, RunConfig, SandboxShell } from './types.js';

/** Port the bridge-backed harness adapters listen on inside the sandbox. */
export const BRIDGE_PORT = 4000;

/**
 * Harness template snapshots expire this long after their last use, so
 * templates for old harness versions do not pile up (the adapter default is
 * never). An expired template is simply rebuilt on the next run.
 */
const TEMPLATE_SNAPSHOT_TTL_MS = 14 * 24 * 60 * 60 * 1000;

/**
 * Environment for every process in the sandbox, including the agent's own
 * shell commands. Pins the `a4` release the installer fetches, keeps
 * `doctor` from warning about newer releases, and keeps benchmark traffic out
 * of product analytics.
 */
export function sandboxEnv(config: RunConfig): Record<string, string> {
  return {
    A4_VERSION: config.a4Version,
    A4_NO_UPDATE_CHECK: '1',
    ARETE_TELEMETRY_DISABLED: '1',
    DO_NOT_TRACK: '1',
  };
}

type RunSandbox = Awaited<ReturnType<typeof createVercelNetworkSandboxSession>>;

/** The `@vercel/sandbox` instance behind a harness network session. */
function nativeSandbox(session: RunSandbox): Sandbox | undefined {
  const native = (session as { sandbox?: Sandbox }).sandbox;
  return native && typeof native.delete === 'function' ? native : undefined;
}

export async function createRunSandbox(
  config: RunConfig,
  template: HarnessSandboxTemplate | undefined,
  runId: string,
): Promise<RunSandbox> {
  const session = await createVercelNetworkSandboxSession({
    ...vercelCredentials(),
    ...(config.sandbox.image ? { image: config.sandbox.image } : { runtime: config.sandbox.runtime }),
    ports: [BRIDGE_PORT],
    timeout: config.sandbox.timeoutMinutes * 60_000,
    resources: { vcpus: config.sandbox.vcpus },
    env: sandboxEnv(config),
    tags: { purpose: 'arete-bench', run: runId.slice(0, 60) },
    snapshotExpiration: TEMPLATE_SNAPSHOT_TTL_MS,
    ...(template ? { template } : {}),
  } as Parameters<typeof createVercelNetworkSandboxSession>[0]);
  // Sandboxes forked from a template come up persistent, and a persistent
  // sandbox snapshots itself (about 1 GB) when it stops. Runs are disposable.
  await nativeSandbox(session)?.update({ persistent: false }).catch(() => {});
  return session;
}

/** Delete a run sandbox along with any snapshot it left behind. */
export async function destroySandbox(session: RunSandbox): Promise<void> {
  const native = nativeSandbox(session);
  if (native) {
    await native.delete({ deleteOrphanSnapshots: true });
  } else {
    await session.destroy();
  }
}

function shellQuote(value: string): string {
  return `'${value.replace(/'/g, `'\\''`)}'`;
}

/**
 * Wrap a sandbox session as a bash runner rooted at `workDir`. Every command
 * runs under `timeout` so a hung verifier cannot stall the run.
 */
export function createShell(
  session: Experimental_SandboxSession,
  workDir: string,
  homeDir: string,
  log?: (line: string) => void,
): SandboxShell {
  return {
    workDir,
    homeDir,
    async run(command, opts = {}): Promise<CommandResult> {
      const timeoutSeconds = opts.timeoutSeconds ?? 120;
      const start = performance.now();
      const wrapped = `cd ${shellQuote(workDir)} && timeout --kill-after=5 ${timeoutSeconds} bash -c ${shellQuote(command)}`;
      const result = await session.run({ command: wrapped });
      const durationMs = Math.round(performance.now() - start);
      log?.(`$ ${command}  [exit ${result.exitCode}, ${durationMs}ms]`);
      return { command, durationMs, ...result };
    },
    async writeText(path, content) {
      await session.writeTextFile({ path, content });
    },
    async readBinary(path) {
      try {
        return await session.readBinaryFile({ path });
      } catch {
        return null;
      }
    },
    async readText(path) {
      try {
        return await session.readTextFile({ path });
      } catch {
        return null;
      }
    },
  };
}

export async function resolveHomeDir(session: Experimental_SandboxSession): Promise<string> {
  const result = await session.run({ command: 'printf %s "$HOME"' });
  return result.stdout.trim() || '/home/vercel-sandbox';
}
