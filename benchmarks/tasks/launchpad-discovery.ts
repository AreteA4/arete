import { parseJson } from '../src/arete-setup.js';
import type { TaskDefinition } from '../src/types.js';
import { check, f1, lastJsonLine } from './lib.js';

interface CatalogEntry {
  slug: string;
  kind: string;
  name?: string;
  protocol?: string;
  modes?: string[];
}

/**
 * The catalog lists one protocol under several slugs (protocol, program and
 * stack), so answers are scored per protocol: any of a protocol's slugs
 * counts, and unknown slugs count as wrong.
 */
function protocolOf(entries: CatalogEntry[]): (slug: string) => string {
  const aliases = new Map<string, string>();
  for (const e of entries) {
    const protocol = e.protocol ?? e.slug;
    for (const alias of [protocol, e.slug, e.name?.replace(/_/g, '-')]) {
      if (alias) aliases.set(alias.toLowerCase(), protocol);
    }
  }
  return (slug) => aliases.get(slug.toLowerCase().trim()) ?? slug.toLowerCase().trim();
}

/**
 * Discovery track: answer a capability question from the live catalog
 * without writing code. Scored per protocol against the catalog's
 * `token-launch` concept at verification time, so the expected answer
 * follows the catalog.
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
  async verify({ shell, finalText }) {
    const result = await shell.run('a4 explore catalog --concept token-launch --json', { timeoutSeconds: 60 });
    const entries = parseJson<{ results?: CatalogEntry[] }>(result.stdout)?.results ?? [];
    const protocol = protocolOf(entries);
    const live = [...new Set(entries.filter((e) => e.modes?.includes('subscribe')).map((e) => protocol(e.slug)))];
    const build = [...new Set(entries.filter((e) => e.modes?.includes('build')).map((e) => protocol(e.slug)))];
    const answer = lastJsonLine<{ live?: string[]; build?: string[] }>(finalText);
    if (!answer) return [check('answer-json', false, 'no JSON object on the final line')];
    const liveScore = f1([...new Set((answer.live ?? []).map(protocol))], live);
    const buildScore = f1([...new Set((answer.build ?? []).map(protocol))], build);
    const fmt = (s: ReturnType<typeof f1>) => `F1 ${s.f1.toFixed(2)} (p ${s.precision.toFixed(2)}, r ${s.recall.toFixed(2)})`;
    return [
      check('answer-json', true, JSON.stringify(answer).slice(0, 300)),
      check('live-slugs', liveScore.f1 >= 0.5, `${fmt(liveScore)} vs [${live.join(', ')}]`),
      check('build-slugs', buildScore.f1 >= 0.5, `${fmt(buildScore)} vs [${build.join(', ')}]`, false),
    ];
  },
};
