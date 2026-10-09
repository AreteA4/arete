import type { TaskDefinition } from '../src/types.js';
import { check, f1, lastJsonLine } from './lib.js';

interface Launchpad {
  /** Has a hosted stack, so it can be streamed live. */
  live: boolean;
  /** Its program SDK carries instruction builders. */
  build: boolean;
  /** Every catalog slug that names it: protocol, program, stack and knowledge entries. */
  slugs: string[];
}

/**
 * The answer key, reviewed against the curated catalog on 2026-10-09. The
 * launchpads are the catalog's protocols in the `launchpad` category; for
 * Raydium that is only its LaunchLab program, whose protocol slug is
 * `raydium`. PumpSwap (`pump-amm`) is the AMM that pump.fun coins graduate
 * into, not a launchpad, even though its catalog entry carries the
 * `token-launch` concept, so naming it counts as wrong.
 *
 * A pinned key keeps scores comparable across runs and harnesses. Update it
 * when a launchpad gains or loses a hosted stack or a program SDK.
 */
export const LAUNCHPADS: Readonly<Record<string, Launchpad>> = {
  jurassic: { live: true, build: true, slugs: ['jurassic', 'jurassic-fi-token-sale', 'jurassic-launchpad'] },
  'meteora-presale': { live: true, build: true, slugs: ['meteora-presale', 'meteora-presale-stream'] },
  'meteora-bonding': { live: false, build: true, slugs: ['meteora-bonding', 'dynamic-bonding-curve'] },
  'pump-fun': { live: false, build: true, slugs: ['pump-fun', 'pumpfun', 'pumpfun-stream'] },
  'raydium-launchpad': { live: false, build: true, slugs: ['raydium-launchpad', 'raydium-launchpad-stream', 'raydium'] },
};

const norm = (slug: string) => slug.toLowerCase().trim().replace(/_/g, '-');

const LAUNCHPAD_BY_SLUG = new Map(
  Object.entries(LAUNCHPADS).flatMap(([id, launchpad]) => launchpad.slugs.map((slug) => [norm(slug), id] as const)),
);

export interface ListScore {
  exact: boolean;
  missing: string[];
  extra: string[];
  f1: ReturnType<typeof f1>;
}

/**
 * Score one answer list per launchpad: any of a launchpad's slugs counts
 * once, and any other slug (unknown or not a launchpad) counts as extra.
 */
export function scoreList(slugs: unknown, mode: 'live' | 'build'): ListScore {
  const given = Array.isArray(slugs) ? slugs.filter((s): s is string => typeof s === 'string') : [];
  const predicted = [...new Set(given.map((s) => LAUNCHPAD_BY_SLUG.get(norm(s)) ?? norm(s)))];
  const expected = Object.entries(LAUNCHPADS).filter(([, launchpad]) => launchpad[mode]).map(([id]) => id);
  const missing = expected.filter((id) => !predicted.includes(id));
  const extra = predicted.filter((id) => !expected.includes(id));
  return { exact: !missing.length && !extra.length, missing, extra, f1: f1(predicted, expected) };
}

function describe(score: ListScore): string {
  const { f1: s } = score;
  const diff = score.exact
    ? 'exact'
    : [score.missing.length && `missing ${score.missing.join(', ')}`, score.extra.length && `extra ${score.extra.join(', ')}`].filter(Boolean).join('; ');
  return `${diff} · F1 ${s.f1.toFixed(2)} (p ${s.precision.toFixed(2)}, r ${s.recall.toFixed(2)})`;
}

/**
 * Discovery track: answer a capability question from the catalog without
 * writing code, scored per launchpad against the answer key above.
 */
export const task: TaskDefinition = {
  name: 'launchpad-discovery',
  description: 'Use the catalog to say which token launchpads Arete can stream and build transactions for',
  track: 'discovery',
  setup: 'initialized',
  // Measured cost profile, used by preflight to estimate a sweep.
  estimate: {
    noCacheTokens: 10,
    cacheReadTokens: 83_438,
    cacheWriteTokens: 12_028,
    outputTokens: 1_317,
    wallMs: 47_000,
    source: 'Claude Code + Sonnet 5.5, 2026-10-08',
  },
  turns: [
    {
      prompt:
        'Which Solana token launchpad protocols can Arete stream live data for right now, and which can it build transactions for? ' +
        'Explain briefly, then put a JSON object on the final line of your answer using Arete catalog slugs: ' +
        '{"live": ["<slug>", ...], "build": ["<slug>", ...]}',
    },
  ],
  async verify({ finalText }) {
    const answer = lastJsonLine<{ live?: unknown; build?: unknown }>(finalText);
    if (!answer) return [check('answer-json', false, 'no JSON object on the final line')];
    const live = scoreList(answer.live, 'live');
    const build = scoreList(answer.build, 'build');
    return [
      check('answer-json', true, JSON.stringify(answer).slice(0, 300)),
      check('live-slugs', live.exact, describe(live)),
      check('build-slugs', build.exact, describe(build), false),
    ];
  },
};
