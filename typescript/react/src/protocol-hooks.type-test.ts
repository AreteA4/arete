// Type-level tests: `npm run typecheck` compiles this file; nothing runs it.
//
// Protocol hooks keep a generated protocol's types: instance states, step
// names and guard codes, participant states and event names.

import {
  createProtocolClient,
  type ProtocolDefinition,
  type ProtocolInteger,
} from '@usearete/sdk';

import {
  useAllowedSteps,
  useProtocolEvents,
  useProtocolInstance,
  useProtocolParticipant,
  useWalletActions,
} from './protocol-hooks';

type Equal<TLeft, TRight> =
  (<T>() => T extends TLeft ? 1 : 2) extends
  (<T>() => T extends TRight ? 1 : 2)
    ? true
    : false;
type Assert<T extends true> = T;

interface RaiseTypes {
  readonly params: { hardCap: ProtocolInteger };
  readonly instanceState: 'open' | 'closed';
  readonly participantState: 'contributed' | 'refunded';
  readonly instanceMetadata: { name?: string };
  readonly participantMetadata: Record<string, never>;
  readonly steps: {
    readonly contribute: {
      readonly scope: 'participant';
      readonly access: 'public';
      readonly args: { amount: ProtocolInteger };
      readonly guardCode: 'raise_not_open';
    };
  };
  readonly events: { readonly 'raise.succeeded': { readonly machine: 'instance' } };
}

declare const RAISE: ProtocolDefinition<RaiseTypes, Record<string, never>, Record<string, never>>;

const publishable = createProtocolClient(RAISE, { auth: { publishableKey: 'a4_pk_test' } });
const secret = createProtocolClient(RAISE, { auth: { secretKey: 'a4_sk_test' } });

export function Component(): null {
  const instance = useProtocolInstance(publishable, 'inst-1');
  const secretInstance = useProtocolInstance(secret, 'inst-1', { wallet: 'wallet' });
  const allowed = useAllowedSteps(publishable, { instance: 'inst-1', wallet: 'wallet' });
  const participant = useProtocolParticipant(secret, { instance: 'inst-1', wallet: null });
  const actions = useWalletActions(publishable, { wallet: undefined, cursor: null, limit: 20 });
  useProtocolEvents(publishable, 'inst-1', {
    onEvent(event) {
      if (event.type === 'emit') {
        const name: 'raise.succeeded' = event.event;
        void name;
      }
    },
  });

  type Assertions = [
    Assert<Equal<NonNullable<typeof instance.instance>['state'], 'open' | 'closed'>>,
    Assert<Equal<NonNullable<typeof secretInstance.instance>['state'], 'open' | 'closed'>>,
    Assert<Equal<NonNullable<typeof participant.data>['state'], 'contributed' | 'refunded' | null>>,
    Assert<Equal<NonNullable<typeof actions.data>['instances'][number]['state'], 'open' | 'closed'>>,
    Assert<Equal<
      NonNullable<typeof actions.data>['instances'][number]['participantState'],
      'contributed' | 'refunded' | null
    >>,
  ];
  const assertions: Assertions | undefined = undefined;
  void assertions;

  const argType: string | undefined = actions.data?.createSteps[0]?.args?.[0]?.type;
  void argType;

  const first = allowed.data?.[0];
  if (first?.step === 'contribute' && first.reason === 'raise_not_open') return null;
  return null;
}
