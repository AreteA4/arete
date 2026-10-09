import 'dotenv/config';
import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';

export const BENCH_ROOT = resolve(import.meta.dirname, '..');

/**
 * Where run directories are written. Point `BENCH_RESULTS_DIR` at a clone of
 * the private results repository to keep transcripts out of this repo.
 */
export function resultsDir(): string {
  // `||`, not `??`: a copied .env.example sets the variable to an empty string.
  return resolve(process.env.BENCH_RESULTS_DIR || resolve(BENCH_ROOT, 'output'));
}

export function vercelCredentials():
  | { token: string; teamId: string; projectId: string }
  | Record<string, never> {
  const { VERCEL_TOKEN, VERCEL_TEAM_ID, VERCEL_PROJECT_ID } = process.env;
  if (VERCEL_TOKEN && VERCEL_TEAM_ID && VERCEL_PROJECT_ID) {
    return { token: VERCEL_TOKEN, teamId: VERCEL_TEAM_ID, projectId: VERCEL_PROJECT_ID };
  }
  // Fall back to VERCEL_OIDC_TOKEN, which @vercel/sandbox reads itself.
  return {};
}

/**
 * Where the shared benchmark agent's key is cached between sweeps: under the
 * package's gitignored `output/`, never in the results directory, which may
 * be a git repository.
 */
export const AGENT_KEY_CACHE = resolve(BENCH_ROOT, 'output/.cache/agent-key');

export function cachedAgentKey(): string | undefined {
  if (!existsSync(AGENT_KEY_CACHE)) return undefined;
  return readFileSync(AGENT_KEY_CACHE, 'utf8').trim() || undefined;
}

export type ModelAuth = 'ai-gateway' | 'direct';

/** `auto` picks the gateway when its credentials are present, otherwise provider keys. */
export function resolveModelAuth(mode: ModelAuth | 'auto'): ModelAuth {
  if (mode !== 'auto') return mode;
  return process.env.AI_GATEWAY_API_KEY || process.env.VERCEL_OIDC_TOKEN ? 'ai-gateway' : 'direct';
}

/** Agent keys available to runs, from `ARETE_AGENT_KEYS` (comma separated). */
export function agentKeys(): string[] {
  return (process.env.ARETE_AGENT_KEYS || process.env.ARETE_AGENT_KEY || '')
    .split(',')
    .map((k) => k.trim())
    .filter(Boolean);
}

export function assertCredentials(modelAuth: ModelAuth): void {
  const missing: string[] = [];
  if (modelAuth === 'ai-gateway' && !process.env.AI_GATEWAY_API_KEY && !process.env.VERCEL_OIDC_TOKEN) {
    missing.push('AI_GATEWAY_API_KEY (or VERCEL_OIDC_TOKEN)');
  }
  if (!('token' in vercelCredentials()) && !process.env.VERCEL_OIDC_TOKEN) {
    missing.push('VERCEL_TOKEN + VERCEL_TEAM_ID + VERCEL_PROJECT_ID');
  }
  if (missing.length) {
    throw new Error(`Missing credentials: ${missing.join(', ')}. See .env.example.`);
  }
}

/** Every secret value that must never reach a written artifact. */
export function knownSecrets(): string[] {
  return [
    ...agentKeys(),
    cachedAgentKey(),
    process.env.AI_GATEWAY_API_KEY,
    process.env.ANTHROPIC_API_KEY,
    process.env.OPENAI_API_KEY,
    process.env.OPENROUTER_API_KEY,
    process.env.VERCEL_OIDC_TOKEN,
    process.env.VERCEL_TOKEN,
  ].filter((v): v is string => typeof v === 'string' && v.length >= 8);
}
