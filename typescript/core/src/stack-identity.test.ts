import { describe, expect, it } from 'vitest';
import { isSameStackSdk } from './stack-identity';
import type { StackDefinition } from './types';

function stack(overrides: Partial<StackDefinition>): StackDefinition {
  return { name: 'presale', endpoints: { ws: 'wss://presale.invalid' }, views: {}, ...overrides };
}

const RELEASE = { stackManifestHash: 'arete:h1:stack-manifest:sha256:aa', liveAlias: 'live' };

describe('isSameStackSdk', () => {
  it('compares package releases when both carry one', () => {
    expect(isSameStackSdk(stack({ packageReleaseHash: 'p1' }), stack({ packageReleaseHash: 'p1' }))).toBe(true);
    expect(
      isSameStackSdk(
        stack({ packageReleaseHash: 'p1', release: RELEASE }),
        stack({ packageReleaseHash: 'p2', release: RELEASE }),
      ),
    ).toBe(false);
  });

  it('falls back to the served release when only one carries a package release', () => {
    // A stack installed on its own carries no package release; the same stack
    // hoisted by a protocol does.
    expect(isSameStackSdk(stack({ release: RELEASE }), stack({ packageReleaseHash: 'p1', release: RELEASE }))).toBe(true);
    expect(
      isSameStackSdk(
        stack({ release: RELEASE }),
        stack({ packageReleaseHash: 'p1', release: { ...RELEASE, liveAlias: 'other' } }),
      ),
    ).toBe(false);
    expect(isSameStackSdk(stack({}), stack({ packageReleaseHash: 'p1' }))).toBe(false);
  });
});
