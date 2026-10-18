// Type-level test of the generated token-raise protocol client
// (`installed-protocol/token-raise/`, exactly as `a4 install protocol`
// writes it) against @usearete/sdk. CI compiles it with tsconfig.strict.json;
// nothing runs it.

import { createProtocolClient, createSession } from '@usearete/sdk';
import TOKEN_RAISE_PROTOCOL, {
  verifyTokenRaiseWebhook,
  type TokenRaiseContributeGuardCode,
  type TokenRaiseEvent,
} from './installed-protocol/token-raise/token-raise.js';

export async function exhaustiveRejections(): Promise<string> {
  const browser = createProtocolClient(TOKEN_RAISE_PROTOCOL, {
    auth: { publishableKey: 'a4_pk_example' },
  });
  const result = await browser.steps.contribute.prepare({
    instance: 'instance-id',
    wallet: 'wallet',
    amount: 1_000_000n,
  });
  if (result.ok) return result.intent.executionId;
  const code = result.rejection.rejected;
  switch (code) {
    case 'deadline_passed':
    case 'hard_cap_exceeded':
    case 'raise_not_open':
    case 'wallet_cap_exceeded':
      return `guard ${code}`;
    case 'step_requires_secret_key':
    case 'step_not_in_session_scope':
    case 'step_not_client_callable':
      return `access ${code}`;
    case 'instance_terminal':
    case 'participant_terminal':
      return `state ${code}`;
    default: {
      const unreachable: never = code;
      return unreachable;
    }
  }
}

export async function accessIsTyped(): Promise<void> {
  const browser = createProtocolClient(TOKEN_RAISE_PROTOCOL, {
    auth: { publishableKey: 'a4_pk_example' },
  });
  // @ts-expect-error `create` is a secret step: a publishable-key client cannot call it.
  await browser.steps.create.prepare({ wallet: 'wallet', deadline: '1', hardCap: '2', members: [], quoteMint: 'mint', softCap: '1', walletCap: '1' });
  // @ts-expect-error u64 arguments are decimal strings or bigints.
  await browser.steps.contribute.prepare({ instance: 'i', wallet: 'w', amount: 1 });

  const backend = createProtocolClient(TOKEN_RAISE_PROTOCOL, {
    auth: { secretKey: 'a4_sk_example' },
  });
  await backend.steps.create.prepare(
    { wallet: 'wallet', deadline: '1', hardCap: '2', members: [], quoteMint: 'mint', softCap: '1', walletCap: '1' },
    { transactionFormat: 'v0' },
  );
}

export function guardCodesAreGenerated(code: TokenRaiseContributeGuardCode, event: TokenRaiseEvent): string {
  return `${code}:${event}`;
}

export function webhooksAreTyped(body: string): string {
  const { payload } = verifyTokenRaiseWebhook('secret', body, { 'arete-webhook-id': 'x' });
  return payload.event;
}

export async function sessionsHoistStacks(): Promise<void> {
  const session = await createSession({ protocols: { tokenRaise: TOKEN_RAISE_PROTOCOL } });
  void session.stacks.meteoraPresale;
  void session.stacks.mirror;
  void session.programs.squadsV4;
  void session.protocols.tokenRaise.steps.contribute;
}
