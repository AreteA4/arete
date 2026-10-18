import { describe, expect, it, vi } from 'vitest';

import { createSession } from '../session';
import type { ProgramSdkDefinition, StackDefinition } from '../types';
import type { AnyProtocolDefinition } from './types';

const PRESALE_PROGRAM = {
  name: 'meteoraPresale',
  programId: 'presSVxnf9UU8jMxhgSMqaRwNiT36qeBdNeTRKjTdbj',
  packageReleaseHash: 'pkg-presale-program-1',
  rawInstructions: {},
} as const satisfies ProgramSdkDefinition;

const SQUADS_PROGRAM = {
  name: 'squadsV4',
  programId: 'SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf',
  packageReleaseHash: 'pkg-squads-1',
  rawInstructions: {},
} as const satisfies ProgramSdkDefinition;

function presaleStack(packageReleaseHash = 'pkg-presale-stack-1'): StackDefinition {
  return {
    name: 'meteora-presale',
    endpoints: { ws: 'wss://presale.invalid' },
    packageReleaseHash,
    views: {},
    programs: { meteoraPresale: PRESALE_PROGRAM },
  };
}

function protocol(
  name: string,
  stacks: Record<string, StackDefinition>,
  programs: Record<string, ProgramSdkDefinition>
): AnyProtocolDefinition {
  return {
    kind: 'protocol',
    name,
    version: '1.0.0',
    package: name,
    packageReleaseHash: `pkg-${name}`,
    protocolSpecHash: `spec-${name}`,
    protocolReleaseHash: `release-${name}`,
    deployment: {
      deploymentId: `dep-${name}`,
      access: 'public',
      apiUrl: 'https://api.example.test/api',
      sessionEndpoint: 'https://api.example.test/ws/sessions',
    },
    instanceVisibility: 'shared',
    steps: {
      contribute: { scope: 'participant', access: 'public', guards: [], args: [] },
      create: { scope: 'create', access: 'secret', guards: [], args: [] },
    },
    events: {},
    metadata: {},
    stacks,
    programs,
  };
}

const OPTIONS = {
  transport: 'http' as const,
  auth: { token: 'jwt' },
  fetch: vi.fn(async () => new Response('null')) as unknown as typeof fetch,
};

describe('sessions with protocols', () => {
  it('connects a stack two protocols share once and hoists their programs without duplicates', async () => {
    const stack = presaleStack();
    const first = protocol('raise', { meteoraPresale: stack }, {
      meteoraPresale: stack.programs!['meteoraPresale']!,
      squadsV4: SQUADS_PROGRAM,
    });
    // A separately generated copy of the same stack package release.
    const copy = presaleStack();
    const second = protocol('vesting', { presale: copy, meteoraPresale: copy }, {
      meteoraPresale: PRESALE_PROGRAM,
      squadsV4: { ...SQUADS_PROGRAM },
    });

    const session = await createSession({ protocols: { raise: first, vesting: second } }, OPTIONS);
    const stacks = session.stacks as Record<string, unknown>;
    const programs = session.programs as Record<string, { programId: string }>;

    expect(Object.keys(stacks).sort()).toEqual(['meteoraPresale', 'presale']);
    expect(stacks['presale']).toBe(stacks['meteoraPresale']);
    expect(session.protocols.raise.stacks['meteoraPresale']).toBe(stacks['meteoraPresale']);
    expect(session.protocols.vesting.stacks['presale']).toBe(stacks['meteoraPresale']);

    // The presale program is served by the connected stack; squads is one standalone member.
    const connectedStack = stacks['meteoraPresale'] as { programs: Record<string, unknown> };
    expect(programs['meteoraPresale']).toBe(connectedStack.programs['meteoraPresale']);
    expect(programs['squadsV4']!.programId).toBe(SQUADS_PROGRAM.programId);
    expect(session.protocols.raise.programs['squadsV4']).toBe(programs['squadsV4']);
    expect(session.protocols.vesting.programs['squadsV4']).toBe(programs['squadsV4']);
    expect(session.protocols.raise.programs['meteoraPresale']).toBe(programs['meteoraPresale']);

    expect(session.protocols.raise.deploymentId).toBe('dep-raise');
    session.close();
  });

  it('shares an explicit stack with a protocol that hoists the same stack', async () => {
    const stack = presaleStack();
    const session = await createSession(
      {
        stacks: { presale: stack },
        protocols: { raise: protocol('raise', { meteoraPresale: presaleStack() }, {}) },
      },
      OPTIONS
    );
    const stacks = session.stacks as Record<string, unknown>;
    expect(stacks['meteoraPresale']).toBe(session.stacks.presale);
    session.close();
  });

  it('throws STACK_KEY_CONFLICT for different stacks under one key', async () => {
    await expect(createSession(
      {
        stacks: { meteoraPresale: presaleStack('pkg-presale-stack-1') },
        protocols: { raise: protocol('raise', { meteoraPresale: presaleStack('pkg-presale-stack-2') }, {}) },
      },
      OPTIONS
    )).rejects.toMatchObject({
      code: 'STACK_KEY_CONFLICT',
      message: expect.stringMatching(/stack 'meteoraPresale' and protocol 'raise'/),
    });
    await expect(createSession(
      {
        protocols: {
          raise: protocol('raise', { meteoraPresale: presaleStack('pkg-presale-stack-1') }, {}),
          vesting: protocol('vesting', { meteoraPresale: presaleStack('pkg-presale-stack-2') }, {}),
        },
      },
      OPTIONS
    )).rejects.toMatchObject({ code: 'STACK_KEY_CONFLICT' });
  });

  it('still throws PROGRAM_KEY_CONFLICT for different programs under one key', async () => {
    const otherSquads = { ...SQUADS_PROGRAM, packageReleaseHash: 'pkg-squads-2' };
    await expect(createSession(
      {
        programs: { squadsV4: otherSquads },
        protocols: { raise: protocol('raise', {}, { squadsV4: SQUADS_PROGRAM }) },
      },
      OPTIONS
    )).rejects.toMatchObject({ code: 'PROGRAM_KEY_CONFLICT' });
    await expect(createSession(
      {
        protocols: {
          raise: protocol('raise', {}, { squadsV4: SQUADS_PROGRAM }),
          vesting: protocol('vesting', {}, { squadsV4: otherSquads }),
        },
      },
      OPTIONS
    )).rejects.toMatchObject({ code: 'PROGRAM_KEY_CONFLICT' });
    // A protocol program that differs from a stack's program under the same key.
    await expect(createSession(
      {
        stacks: { presale: presaleStack() },
        protocols: {
          raise: protocol('raise', {}, {
            meteoraPresale: { ...PRESALE_PROGRAM, packageReleaseHash: 'pkg-presale-program-2' },
          }),
        },
      },
      OPTIONS
    )).rejects.toMatchObject({ code: 'PROGRAM_KEY_CONFLICT' });
  });

  it('accepts a session of protocols only', async () => {
    const session = await createSession({ protocols: { raise: protocol('raise', {}, {}) } }, OPTIONS);
    expect(Object.keys(session.stacks)).toEqual([]);
    expect(session.protocols.raise.keyClass).toBe('publishable');
    expect(() => session.chain.nativeBalance).toThrow(/no chain/);
    await expect(session.transaction([])).rejects.toMatchObject({ code: 'INVALID_CONFIG' });
    session.close();
  });

  it('types and runs session protocols as secret clients with a secret key', async () => {
    const session = await createSession(
      { protocols: { raise: protocol('raise', {}, {}) } },
      { ...OPTIONS, auth: { secretKey: 'a4_sk_test' } }
    );
    expect(session.protocols.raise.keyClass).toBe('secret');
    expect(Object.keys(session.protocols.raise.steps).sort()).toEqual(['contribute', 'create']);
    session.close();
  });
});
