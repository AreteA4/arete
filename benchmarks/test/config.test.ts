import assert from 'node:assert/strict';
import { resolve } from 'node:path';
import { describe, test } from 'node:test';
import { compareVersions, resolveA4Version, stalePinWarning } from '../src/a4-version.js';
import { prepareSweep } from '../src/cli/args.js';
import { expandSweep, SweepConfigSchema } from '../src/config.js';
import { BENCH_ROOT } from '../src/env.js';
import { runLabel } from '../src/history.js';
import { a4VersionCheck, keyModeTaskChecks } from '../src/preflight.js';
import type { RunConfig, TaskDefinition } from '../src/types.js';

// No test here touches the network: npm `latest` comes from these stubs.
const latest = (version: string) => async () => version;
const offline = async (): Promise<string> => {
  throw new Error('offline');
};
const config = (name: string) => resolve(BENCH_ROOT, 'configs', name);

describe('a4 version resolution', () => {
  test('latest resolves to the npm release', async () => {
    assert.deepEqual(await resolveA4Version('latest', latest('0.33.0')), { version: '0.33.0', source: 'latest', latest: '0.33.0' });
  });

  test('latest without the registry fails instead of guessing', async () => {
    await assert.rejects(resolveA4Version('latest', offline), /pin one with --a4-version/);
  });

  test('a pin is kept, and still works offline', async () => {
    assert.deepEqual(await resolveA4Version('v0.32.0', latest('0.33.0')), { version: '0.32.0', source: 'pinned', latest: '0.33.0' });
    const resolved = await resolveA4Version('0.32.0', offline);
    assert.equal(resolved.version, '0.32.0');
    assert.equal(resolved.lookupError, 'offline');
  });

  test('a malformed pin is rejected', async () => {
    await assert.rejects(resolveA4Version('0.33', latest('0.33.0')), /a4Version must be/);
  });

  test('stale pins warn; current and newer ones do not', async () => {
    assert.equal(compareVersions('0.32.0', '0.33.0'), -1);
    assert.equal(compareVersions('0.10.0', '0.9.9'), 1);
    assert.equal(compareVersions('0.33.0-rc.1', '0.33.0'), 0);
    assert.match(stalePinWarning(await resolveA4Version('0.32.0', latest('0.33.0'))) ?? '', /0\.32\.0 is behind the latest release 0\.33\.0/);
    assert.equal(stalePinWarning(await resolveA4Version('0.33.0', latest('0.33.0'))), undefined);
    assert.equal(stalePinWarning(await resolveA4Version('latest', latest('0.33.0'))), undefined);
  });

  test('preflight reports the version as a non-blocking check', async () => {
    const stale = a4VersionCheck(await resolveA4Version('0.32.0', latest('0.33.0')));
    assert.equal(stale.ok, false);
    assert.equal(stale.blocking, false);
    const current = a4VersionCheck(await resolveA4Version('latest', latest('0.33.0')));
    assert.equal(current.ok, true);
    assert.match(current.detail, /0\.33\.0/);
  });
});

describe('sweep configs', () => {
  test('a4Version defaults to latest and keyMode to pool', () => {
    const sweep = SweepConfigSchema.parse({ agents: [{ harness: 'codex', model: 'openai/gpt-6.1-sol' }], tasks: ['onboarding.ts'] });
    assert.equal(sweep.a4Version, 'latest');
    assert.equal(sweep.keyMode, 'pool');
  });

  test('an unknown key mode is rejected', () => {
    assert.throws(() => SweepConfigSchema.parse({ agents: [{ harness: 'codex', model: 'm' }], tasks: ['t.ts'], keyMode: 'shared' }));
  });

  test('expanding needs a resolved version', () => {
    const sweep = SweepConfigSchema.parse({ agents: [{ harness: 'codex', model: 'm' }], tasks: ['t.ts'] });
    assert.throws(() => expandSweep(sweep), /resolve a4Version/);
    const runs = expandSweep(sweep, { version: '0.33.0', source: 'latest', latest: '0.33.0' });
    assert.equal(runs[0]!.a4Version, '0.33.0');
    assert.equal(runs[0]!.a4VersionSource, 'latest');
  });

  test('the weekly matrix follows the latest release', async () => {
    const { runs, a4 } = await prepareSweep([config('matrix.json')], undefined, latest('0.34.1'));
    assert.equal(a4.source, 'latest');
    assert.ok(runs.every((r) => r.a4Version === '0.34.1' && r.keyMode === 'pool'));
  });

  test('--a4-version pins over the config', async () => {
    const { runs, a4 } = await prepareSweep([config('matrix.json'), '--a4-version', '0.32.0'], undefined, latest('0.34.1'));
    assert.deepEqual([a4.source, a4.version], ['pinned', '0.32.0']);
    assert.ok(runs.every((r) => r.a4Version === '0.32.0' && r.a4VersionSource === 'pinned'));
  });

  test('the fresh-onboarding config runs only onboarding, without keys', async () => {
    const { sweep, runs } = await prepareSweep([config('fresh-onboarding.json')], undefined, latest('0.33.0'));
    assert.equal(sweep.keyMode, 'fresh');
    assert.deepEqual([...new Set(runs.map((r) => r.task))], ['onboarding.ts']);
    assert.ok(runs.length <= 5, 'stays under the 5/hour/IP signup limit');
  });

  test('--key-mode fresh works with inline flags', async () => {
    const { runs } = await prepareSweep(
      ['--task', 'onboarding.ts', '--harness', 'claude-code', '--model', 'anthropic/claude-sonnet-5.5', '--key-mode', 'fresh'],
      undefined,
      latest('0.33.0'),
    );
    assert.equal(runs.length, 1);
    assert.equal(runs[0]!.keyMode, 'fresh');
  });
});

describe('fresh key mode', () => {
  const task = (setup: TaskDefinition['setup']) => ({ setup }) as TaskDefinition;
  const run = (keyMode: RunConfig['keyMode'], name: string) => ({ keyMode, task: name }) as RunConfig;
  const tasks = new Map([
    ['onboarding.ts', task('bare')],
    ['ore-live-round.ts', task('initialized')],
  ]);

  test('preflight blocks fresh runs of initialized tasks', () => {
    assert.deepEqual(keyModeTaskChecks([run('fresh', 'onboarding.ts')], tasks), []);
    assert.deepEqual(keyModeTaskChecks([run('pool', 'ore-live-round.ts')], tasks), []);
    const [blocked] = keyModeTaskChecks([run('fresh', 'onboarding.ts'), run('fresh', 'ore-live-round.ts')], tasks);
    assert.equal(blocked?.blocking, true);
    assert.match(blocked?.detail ?? '', /ore-live-round\.ts/);
  });

  test('fresh runs get their own comparison rows', () => {
    const onboarding = { name: 'onboarding', track: 'onboarding' as const, setup: 'bare' as const };
    assert.equal(runLabel({ task: onboarding, config: { keyMode: 'pool' } as RunConfig }), 'onboarding');
    assert.equal(runLabel({ task: onboarding, config: { keyMode: 'fresh' } as RunConfig }), 'onboarding [fresh account]');
  });
});
