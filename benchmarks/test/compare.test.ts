import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { compareTable } from '../src/cli/compare.js';
import { taskLabel } from '../src/history.js';
import type { RunReport } from '../src/types.js';

/** The fields compareTable reads; the rest of a report does not matter here. */
function report(task: RunReport['task'], passed: boolean): RunReport {
  return {
    task,
    status: 'success',
    config: { harness: 'codex', model: 'openai/gpt-6.1-sol' },
    verification: { passed, score: passed ? 1 : 0, checks: [] },
    timing: { totalMs: 60_000, milestones: {} },
    tokens: { inputTokens: 1000, outputTokens: 100, peakContextTokens: 0 },
    cost: { modelUsd: 0.1 },
    tools: { total: 5, failed: 0, a4Commands: {}, helpLookups: 0, mcpCalls: {} },
  } as unknown as RunReport;
}

const rows = (table: string) => table.split('\n').slice(2);

describe('grading versions', () => {
  test('reports without a grading are grading 1', () => {
    assert.equal(taskLabel({ name: 'launchpad-discovery', track: 'discovery', setup: 'initialized' }), 'launchpad-discovery');
    assert.equal(
      taskLabel({ name: 'launchpad-discovery', track: 'discovery', setup: 'initialized', grading: 2 }),
      'launchpad-discovery (grading 2)',
    );
  });

  test('compare never pools runs graded under different rules', () => {
    const base = { name: 'launchpad-discovery', track: 'discovery', setup: 'initialized' } as const;
    const table = compareTable([
      report(base, true),
      report(base, true),
      report({ ...base, grading: 1 }, false),
      report({ ...base, grading: 2 }, false),
    ]);
    const [old, current] = rows(table);
    assert.equal(rows(table).length, 2);
    assert.match(old!, /^\| launchpad-discovery \| codex \| openai\/gpt-6\.1-sol \| 3 \| 0 \| 2\/3 \|/);
    assert.match(current!, /^\| launchpad-discovery \(grading 2\) \| codex \| openai\/gpt-6\.1-sol \| 1 \| 0 \| 0\/1 \|/);
  });
});
