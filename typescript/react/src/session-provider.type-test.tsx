// Type-level tests: `npm run typecheck` compiles this file; nothing runs it.
//
// `useAreteSession<typeof DEFINITION>()` types `session.programs.<key>` the
// way `createSession(DEFINITION)` does, and the session it gives is a
// `useOperationMutation` executor.

import React from 'react';
import type { ProgramSdkDefinition, createSession } from '@usearete/sdk';

import { useOperationMutation } from './hooks/use-operation-mutation';
import { AreteSessionProvider, useAreteSession } from './session-provider';

type Equal<TLeft, TRight> =
  (<T>() => T extends TLeft ? 1 : 2) extends
  (<T>() => T extends TRight ? 1 : 2)
    ? true
    : false;
type Assert<T extends true> = T;

declare const SQUADS_PROGRAM: ProgramSdkDefinition & { readonly name: 'squads' };
const DEFINITION = { programs: { squads: SQUADS_PROGRAM } };

type Opened = Awaited<ReturnType<typeof createSession<typeof DEFINITION>>>;

type ByDefinition = NonNullable<ReturnType<typeof useAreteSession<typeof DEFINITION>>['session']>;
export type ProgramIsTyped = Assert<Equal<
  ByDefinition['programs']['squads'],
  Opened['programs']['squads']
>>;
type BySession = NonNullable<ReturnType<typeof useAreteSession<Opened>>['session']>;
export type SessionTypeIsKept = Assert<Equal<BySession, Opened>>;

export function TypedByDefinition() {
  const { session, ready } = useAreteSession<typeof DEFINITION>();
  // @ts-expect-error not a member of the definition
  void session?.programs.missing;
  const run = useOperationMutation(async () => {
    throw new Error('unused');
  }, { executor: ready });
  return run.status;
}

declare const opened: Opened;

export const owned = (
  <AreteSessionProvider definition={DEFINITION} options={{ auth: { publishableKey: 'pk' } }}>
    {null}
  </AreteSessionProvider>
);
export const shared = <AreteSessionProvider session={opened}>{null}</AreteSessionProvider>;
// @ts-expect-error a provider shares a session or opens one, not both
export const both = <AreteSessionProvider session={opened} definition={DEFINITION}>{null}</AreteSessionProvider>;
// @ts-expect-error the wallet is a prop, so changing it does not reopen the session
export const walletOption = <AreteSessionProvider definition={DEFINITION} options={{ wallet: undefined }}>{null}</AreteSessionProvider>;
