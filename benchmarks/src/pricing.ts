import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { BENCH_ROOT } from './env.js';
import type { UsageRecord } from './types.js';

const MODELS_URL = 'https://ai-gateway.vercel.sh/v1/models';
const CACHE_PATH = resolve(BENCH_ROOT, 'output/.cache/gateway-models.json');
const CACHE_TTL_MS = 24 * 60 * 60 * 1000;

/** Per-token USD prices as published by the AI Gateway model catalog. */
export interface ModelPricing {
  input: number;
  output: number;
  cacheRead?: number;
  cacheWrite?: number;
  contextWindow?: number;
}

interface GatewayModel {
  id: string;
  context_window?: number;
  pricing?: Record<string, unknown>;
}

let catalog: Map<string, ModelPricing> | undefined;

function num(value: unknown): number | undefined {
  const n = typeof value === 'string' ? Number(value) : typeof value === 'number' ? value : NaN;
  return Number.isFinite(n) ? n : undefined;
}

async function fetchCatalog(): Promise<GatewayModel[]> {
  if (existsSync(CACHE_PATH) && Date.now() - statSync(CACHE_PATH).mtimeMs < CACHE_TTL_MS) {
    return JSON.parse(readFileSync(CACHE_PATH, 'utf8')) as GatewayModel[];
  }
  const response = await fetch(MODELS_URL);
  if (!response.ok) throw new Error(`gateway model catalog: HTTP ${response.status}`);
  const body = (await response.json()) as { data?: GatewayModel[] };
  const models = body.data ?? [];
  mkdirSync(dirname(CACHE_PATH), { recursive: true });
  writeFileSync(CACHE_PATH, JSON.stringify(models));
  return models;
}

export async function loadPricing(): Promise<Map<string, ModelPricing>> {
  if (catalog) return catalog;
  catalog = new Map();
  try {
    for (const model of await fetchCatalog()) {
      const input = num(model.pricing?.input);
      const output = num(model.pricing?.output);
      if (input === undefined || output === undefined) continue;
      catalog.set(model.id, {
        input,
        output,
        cacheRead: num(model.pricing?.input_cache_read),
        cacheWrite: num(model.pricing?.input_cache_write),
        contextWindow: model.context_window,
      });
    }
  } catch (err) {
    process.stderr.write(`warning: could not load gateway pricing: ${String(err)}\n`);
  }
  return catalog;
}

/** Normalise a harness model id (`gpt-5.6-sol`) to its gateway id. */
export function gatewayModelId(model: string, harness: string): string {
  if (model.includes('/')) return model;
  if (harness === 'codex') return `openai/${model}`;
  if (harness === 'claude-code') return `anthropic/${model}`;
  return model;
}

export function modelCost(pricing: ModelPricing | undefined, usage: UsageRecord): number | undefined {
  if (!pricing) return undefined;
  return (
    usage.noCacheTokens * pricing.input +
    usage.cacheReadTokens * (pricing.cacheRead ?? pricing.input) +
    usage.cacheWriteTokens * (pricing.cacheWrite ?? pricing.input) +
    usage.outputTokens * pricing.output
  );
}

/** Vercel Sandbox Pro rates: $0.128 per vCPU-hour, $0.0212 per GB-hour (2 GB per vCPU). */
export function sandboxCostUpperBound(vcpus: number, wallMs: number): number {
  const hours = wallMs / 3_600_000;
  return vcpus * hours * 0.128 + vcpus * 2 * hours * 0.0212;
}
