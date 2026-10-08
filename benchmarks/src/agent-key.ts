import { chmodSync, mkdirSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';
import { fetchAgentProfile, installA4 } from './arete-setup.js';
import { AGENT_KEY_CACHE, cachedAgentKey } from './env.js';
import { createRunSandbox, createShell, resolveHomeDir } from './sandbox.js';
import type { RunConfig } from './types.js';

/**
 * Return a working agent key for runs when `ARETE_AGENT_KEYS` is empty:
 * the cached one if Arete still accepts it, otherwise a new trial agent
 * signed up once in a throwaway sandbox. One agent per sweep (and across
 * sweeps) avoids a signup per run and the 5/hour/IP signup limit.
 */
export async function sharedAgentKey(config: RunConfig, log: (line: string) => void): Promise<string> {
  const cached = cachedAgentKey();
  if (cached && (await fetchAgentProfile(cached))) {
    log(`using cached benchmark agent (${AGENT_KEY_CACHE})`);
    return cached;
  }

  log('signing up a benchmark agent in a throwaway sandbox…');
  const sandbox = await createRunSandbox({ ...config, sandbox: { ...config.sandbox, timeoutMinutes: 10 } }, undefined, 'agent-signup');
  try {
    const shell = createShell(sandbox, '/tmp', await resolveHomeDir(sandbox));
    await installA4(shell, config.a4Version);
    const signup = await shell.run('a4 --profile agent auth signup arete-bench --json', { timeoutSeconds: 60 });
    if (signup.exitCode === 75) {
      throw new Error('Arete agent signup is rate limited (5/hour/IP). Retry later or set ARETE_AGENT_KEYS.');
    }
    if (signup.exitCode !== 0) {
      throw new Error(`agent signup failed (exit ${signup.exitCode}): ${signup.stderr || signup.stdout}`);
    }
    const credentials = (await shell.readText(`${shell.homeDir}/.arete/credentials.toml`)) ?? '';
    const key = /"(a4_ak_[A-Za-z0-9]+)"/.exec(credentials)?.[1];
    if (!key) throw new Error('agent signup succeeded but no key was found in the credentials file');
    mkdirSync(dirname(AGENT_KEY_CACHE), { recursive: true });
    writeFileSync(AGENT_KEY_CACHE, `${key}\n`, { mode: 0o600 });
    chmodSync(AGENT_KEY_CACHE, 0o600);
    log(`signed up a benchmark agent; key cached at ${AGENT_KEY_CACHE}`);
    return key;
  } finally {
    await Promise.resolve(sandbox.destroy()).catch(() => {});
  }
}
