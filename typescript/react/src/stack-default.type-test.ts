import type { ViewDef } from '@usearete/sdk';

import { useArete } from './stack';

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
    };
  };
};

declare module './stack' {
  interface AreteDefaultStackRegistry {
    defaultStack: KeyedStack;
  }
}

type Equal<TLeft, TRight> =
  (<T>() => T extends TLeft ? 1 : 2) extends
  (<T>() => T extends TRight ? 1 : 2)
    ? true
    : false;
type Assert<T extends true> = T;

const arete = useArete();

type RegisteredStateUse = typeof arete.views.Round.state.use;
type RegisteredRefresh = typeof arete.views.Round.state.refresh;

export type DefaultStackStateUseIsTyped = Assert<
  Equal<Parameters<RegisteredStateUse>[0], RoundKey | null | undefined>
>;
export type DefaultStackRefreshIsTyped = Assert<
  Equal<Parameters<RegisteredRefresh>[0], RoundKey | null | undefined>
>;
