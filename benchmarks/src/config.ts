import { readFileSync } from 'node:fs';
import { z } from 'zod';
import { resolveModelAuth } from './env.js';
import type { RunConfig } from './types.js';

const harnessSchema = z.enum(['claude-code', 'codex', 'opencode']);
const effortSchema = z.enum(['low', 'medium', 'high', 'xhigh', 'max']);

const sandboxSchema = z
  .object({
    image: z.string().optional(),
    runtime: z.string().optional(),
    vcpus: z.number().int().min(1).max(8).default(2),
    timeoutMinutes: z.number().int().min(5).max(24 * 60).default(45),
  })
  .prefault({})
  .transform((s) => (s.image || s.runtime ? s : { ...s, image: 'vercel/sandbox/universal' }));

/** Settings shared by single runs and every cell of a sweep. */
const sharedSchema = z.object({
  a4Version: z.string().default('0.32.0'),
  skillsRef: z.string().optional(),
  keyMode: z.enum(['pool', 'signup']).default('pool'),
  modelAuth: z.enum(['auto', 'ai-gateway', 'direct']).default('auto'),
  instructions: z.string().optional(),
  turnTimeoutMinutes: z.number().min(1).default(20),
  sandbox: sandboxSchema,
});

const agentSchema = z.object({
  harness: harnessSchema,
  model: z.string(),
  label: z.string().optional(),
  effort: effortSchema.optional(),
});

export const RunConfigSchema = sharedSchema.extend({
  ...agentSchema.shape,
  task: z.string(),
});

/** A sweep runs every agent against every task `repetitions` times. */
export const SweepConfigSchema = sharedSchema.extend({
  agents: z.array(agentSchema).min(1),
  tasks: z.array(z.string()).min(1),
  repetitions: z.number().int().min(1).default(1),
  concurrency: z.number().int().min(1).max(32).default(4),
});

export type SweepConfig = z.infer<typeof SweepConfigSchema>;

function readJson(path: string): unknown {
  return JSON.parse(readFileSync(path, 'utf8'));
}

/** Load either a single-run config or a sweep config from disk. */
export function loadConfigFile(path: string): SweepConfig {
  const raw = readJson(path) as Record<string, unknown>;
  if (Array.isArray(raw.agents)) return SweepConfigSchema.parse(raw);
  const run = RunConfigSchema.parse(raw);
  const { harness, model, label, effort, task, ...shared } = run;
  return SweepConfigSchema.parse({
    ...shared,
    agents: [{ harness, model, label, effort }],
    tasks: [task],
  });
}

export function expandSweep(sweep: SweepConfig): RunConfig[] {
  const runs: RunConfig[] = [];
  for (let rep = 0; rep < sweep.repetitions; rep++) {
    for (const task of sweep.tasks) {
      for (const agent of sweep.agents) {
        runs.push({
          ...agent,
          task,
          a4Version: sweep.a4Version,
          skillsRef: sweep.skillsRef,
          keyMode: sweep.keyMode,
          modelAuth: resolveModelAuth(sweep.modelAuth),
          instructions: sweep.instructions,
          turnTimeoutMinutes: sweep.turnTimeoutMinutes,
          sandbox: sweep.sandbox,
        });
      }
    }
  }
  return runs;
}
