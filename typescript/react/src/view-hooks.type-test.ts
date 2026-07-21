import type { ViewDef } from '@usearete/sdk';

import type { UseAreteResult } from './stack';
import type { ListViewHookOptions, StateViewHook, StateViewHookOptions } from './types';
import { createStateViewHook } from './view-hooks';

interface RoundEntity {
  roundId: bigint;
}

interface RoundKey {
  roundId: bigint;
}

type KeyedStack = {
  readonly name: 'keyed';
  readonly endpoints: { readonly ws: 'wss://example.invalid' };
  readonly views: {
    readonly Round: {
      readonly state: ViewDef<RoundEntity, 'state', RoundKey>;
      readonly list: ViewDef<RoundEntity, 'list'>;
    };
  };
};

type Equal<TLeft, TRight> =
  (<T>() => T extends TLeft ? 1 : 2) extends
  (<T>() => T extends TRight ? 1 : 2)
    ? true
    : false;
type Assert<T extends true> = T;

type GeneratedUse = UseAreteResult<KeyedStack, undefined>['views']['Round']['state']['use'];
type PublicUse = StateViewHook<RoundEntity, RoundKey>['use'];
type FactoryUse = ReturnType<typeof createStateViewHook<RoundEntity, RoundKey>>['use'];
type GeneratedStateResult = ReturnType<PublicUse>;
type ReadyStateWithData = Extract<GeneratedStateResult, { status: 'ready'; isEmpty: false }>;
type ReadyEmptyState = Extract<GeneratedStateResult, { status: 'ready'; isEmpty: true }>;

export type GeneratedStateKeyIsInferred = Assert<
  Equal<Parameters<GeneratedUse>[0], RoundKey | null | undefined>
>;
export type GeneratedStateKeyIsRequired = Assert<
  Equal<[] extends Parameters<GeneratedUse> ? true : false, false>
>;
export type PublicStateKeyMatchesGeneratedKey = Assert<
  Equal<Parameters<PublicUse>[0], RoundKey | null | undefined>
>;
export type StateHookFactoryPreservesKey = Assert<
  Equal<Parameters<FactoryUse>[0], RoundKey | null | undefined>
>;
export type WrongStateKeyIsRejected = Assert<
  Equal<{ authority: string } extends Parameters<GeneratedUse>[0] ? true : false, false>
>;
export type ReadyStateDataIsNarrowed = Assert<
  Equal<ReadyStateWithData['data'], RoundEntity>
>;
export type EmptyStateDataIsUndefined = Assert<
  Equal<ReadyEmptyState['data'], undefined>
>;
export type StateInitialDataIsTyped = Assert<
  Equal<StateViewHookOptions<RoundEntity>['initialData'], RoundEntity | undefined>
>;
export type ListInitialDataIsTyped = Assert<
  Equal<
    ListViewHookOptions<RoundEntity>['initialData'],
    readonly RoundEntity[] | undefined
  >
>;
