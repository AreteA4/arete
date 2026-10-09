import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { scoreList, task } from '../tasks/launchpad-discovery.js';
import type { VerifyContext } from '../src/types.js';

// Answers below are taken from earlier benchmark runs.
describe('launchpad answer key', () => {
  test('protocol, program and stack slugs all name the same launchpad', () => {
    assert.ok(scoreList(['jurassic', 'meteora-presale'], 'live').exact);
    assert.ok(scoreList(['jurassic-launchpad', 'meteora-presale'], 'live').exact);
    assert.ok(
      scoreList(['jurassic-fi-token-sale', 'meteora-bonding', 'meteora-presale', 'pumpfun', 'raydium-launchpad'], 'build').exact,
    );
    assert.ok(scoreList(['jurassic', 'meteora-presale', 'meteora-bonding', 'pump-fun', 'raydium'], 'build').exact);
  });

  test('two slugs for one launchpad count once', () => {
    const score = scoreList(
      ['pumpfun', 'raydium-launchpad', 'meteora-bonding', 'jurassic-fi-token-sale', 'jurassic-launchpad', 'meteora-presale', 'raydium'],
      'build',
    );
    assert.ok(score.exact, JSON.stringify(score));
  });

  test('PumpSwap is not a launchpad', () => {
    const score = scoreList(
      ['jurassic-fi-token-sale', 'meteora-bonding', 'meteora-presale', 'pump-amm', 'pumpfun', 'raydium-launchpad'],
      'build',
    );
    assert.equal(score.exact, false);
    assert.deepEqual(score.extra, ['pump-amm']);
    assert.deepEqual(score.missing, []);
  });

  test('a launchpad without a hosted stack is not live', () => {
    const score = scoreList(['jurassic', 'meteora-presale', 'pumpfun'], 'live');
    assert.deepEqual(score.extra, ['pump-fun']);
    assert.deepEqual(scoreList(['meteora_presale'], 'live').missing, ['jurassic']);
  });

  test('non-string and missing lists score as empty', () => {
    assert.deepEqual(scoreList(undefined, 'live').missing, ['jurassic', 'meteora-presale']);
    assert.deepEqual(scoreList([42, null, 'jurassic'], 'live').missing, ['meteora-presale']);
  });
});

describe('launchpad-discovery verify', () => {
  const verify = (finalText: string) => task.verify({ finalText } as VerifyContext);

  test('needs the JSON object on the final line', async () => {
    const checks = await verify('Jurassic and Meteora Presale stream live.');
    assert.deepEqual(checks.map((c) => [c.id, c.passed]), [['answer-json', false]]);
  });

  test('live is required, build only scores', async () => {
    const checks = await verify(
      'Two stream live; five can build.\n' +
        '{"live":["jurassic-launchpad","meteora-presale"],"build":["jurassic-fi-token-sale","meteora-bonding","meteora-presale","pump-amm","pumpfun","raydium-launchpad"]}',
    );
    const byId = Object.fromEntries(checks.map((c) => [c.id, c]));
    assert.equal(byId['live-slugs']?.passed, true);
    assert.equal(byId['live-slugs']?.required, true);
    assert.equal(byId['build-slugs']?.passed, false);
    assert.equal(byId['build-slugs']?.required, false);
    assert.match(byId['build-slugs']!.detail, /extra pump-amm/);
  });
});
