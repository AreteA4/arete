// Type-level tests: `npm run typecheck` compiles this file; nothing runs it.
//
// The module below is written exactly as `a4 install protocol` generates a
// protocol module (token-raise), with stub stack and program definitions in
// place of the generated ones and `../index` in place of `@usearete/sdk`.

import {
  createProtocolClient,
  createSession,
  verifyWebhook as verifyProtocolWebhook,
  type ProgramInterface,
  type ProtocolAccessRejectionCode,
  type ProtocolStateRejectionCode,
  type ProtocolDefinition,
  type ProtocolInteger,
  type ProtocolWebhookPayload,
  type VerifiedWebhook,
  type VerifyWebhookOptions,
  type WebhookHeaders,
} from '../index';
import type { ConnectedArete } from '../client';

// --- stubs for the generated stack and program modules ---------------------

const METEORA_PRESALE_STACK = {
  name: 'meteora-presale',
  endpoints: { ws: 'wss://meteora-presale.invalid' },
  packageReleaseHash: 'arete:registry-package-release:v2:sha256:stack',
  views: {},
  programs: {
    meteoraPresale: {
      name: 'meteoraPresale',
      programId: 'presSVxnf9UU8jMxhgSMqaRwNiT36qeBdNeTRKjTdbj',
      packageReleaseHash: 'arete:registry-package-release:v2:sha256:presale-program',
    },
  },
} as const;

const SQUADS_V4_PROGRAM = {
  name: 'squadsV4',
  programId: 'SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf',
  packageReleaseHash: 'arete:registry-package-release:v2:sha256:squads',
} as const;

// --- generated module (token-raise) -----------------------------------------

export type TokenRaiseInstanceState = 'awaiting_approval' | 'closed' | 'failed' | 'finalized' | 'open' | 'succeeded';
export type TokenRaiseParticipantState = 'claimed' | 'contributed' | 'refunded';
export interface TokenRaiseParams { deadline: ProtocolInteger; hardCap: ProtocolInteger; quoteMint: string; softCap: ProtocolInteger; walletCap: ProtocolInteger; }
export interface TokenRaiseCreateArgs { deadline: ProtocolInteger; hardCap: ProtocolInteger; members: readonly string[]; quoteMint: string; softCap: ProtocolInteger; walletCap: ProtocolInteger; }
export type TokenRaiseCreateGuardCode = 'soft_cap_above_hard_cap';
export interface TokenRaiseContributeArgs { amount: ProtocolInteger; }
export type TokenRaiseContributeGuardCode = 'deadline_passed' | 'hard_cap_exceeded' | 'raise_not_open' | 'wallet_cap_exceeded';
// eslint-disable-next-line @typescript-eslint/no-empty-interface
export interface TokenRaiseClaimArgs {}
export type TokenRaiseClaimGuardCode = 'raise_not_finalized';
// eslint-disable-next-line @typescript-eslint/no-empty-interface
export interface TokenRaiseRefundArgs {}
export type TokenRaiseRefundGuardCode = 'raise_not_failed';
export interface TokenRaiseInstanceMetadata { name?: string; tagline?: string; ticker?: string; }
// eslint-disable-next-line @typescript-eslint/no-empty-interface
export interface TokenRaiseParticipantMetadata {}
export type TokenRaiseEvent = 'contribution.confirmed' | 'raise.failed' | 'raise.succeeded';

export interface TokenRaiseProtocolTypes {
  readonly params: TokenRaiseParams;
  readonly instanceState: TokenRaiseInstanceState;
  readonly participantState: TokenRaiseParticipantState;
  readonly instanceMetadata: TokenRaiseInstanceMetadata;
  readonly participantMetadata: TokenRaiseParticipantMetadata;
  readonly steps: {
    readonly create: { readonly scope: 'create'; readonly access: 'secret'; readonly args: TokenRaiseCreateArgs; readonly guardCode: TokenRaiseCreateGuardCode };
    readonly contribute: { readonly scope: 'participant'; readonly access: 'public'; readonly args: TokenRaiseContributeArgs; readonly guardCode: TokenRaiseContributeGuardCode };
    readonly claim: { readonly scope: 'participant'; readonly access: 'public'; readonly args: TokenRaiseClaimArgs; readonly guardCode: TokenRaiseClaimGuardCode };
    readonly refund: { readonly scope: 'participant'; readonly access: 'public'; readonly args: TokenRaiseRefundArgs; readonly guardCode: TokenRaiseRefundGuardCode };
  };
  readonly events: {
    readonly 'contribution.confirmed': { readonly machine: 'participant' };
    readonly 'raise.failed': { readonly machine: 'instance' };
    readonly 'raise.succeeded': { readonly machine: 'instance' };
  };
}

const STACKS = { meteoraPresale: METEORA_PRESALE_STACK } as const;
const PROGRAMS = { meteoraPresale: METEORA_PRESALE_STACK.programs.meteoraPresale, squadsV4: SQUADS_V4_PROGRAM } as const;

export const TOKEN_RAISE_PROTOCOL: ProtocolDefinition<TokenRaiseProtocolTypes, typeof STACKS, typeof PROGRAMS> = {
  kind: 'protocol',
  name: 'tokenRaise',
  version: '1.0.0',
  package: 'token-raise',
  packageReleaseHash: 'arete:registry-package-release:v2:sha256:protocol',
  protocolSpecHash: 'arete:h1:protocol-spec:sha256:spec',
  protocolReleaseHash: 'arete:h1:protocol-release:sha256:release',
  deployment: { deploymentId: '00000000-0000-4000-8000-000000000001', access: 'public', apiUrl: 'https://api.arete.run/api', sessionEndpoint: 'https://api.arete.run/ws/sessions' },
  instanceVisibility: 'account',
  steps: {
    create: { scope: 'create', access: 'secret', guards: ['soft_cap_above_hard_cap'], args: ['deadline', 'hardCap', 'members', 'quoteMint', 'softCap', 'walletCap'] },
    contribute: { scope: 'participant', access: 'public', guards: ['raise_not_open', 'deadline_passed', 'wallet_cap_exceeded', 'hard_cap_exceeded'], args: ['amount'] },
    claim: { scope: 'participant', access: 'public', guards: ['raise_not_finalized'], args: [] },
    refund: { scope: 'participant', access: 'public', guards: ['raise_not_failed'], args: [] },
  },
  events: { 'contribution.confirmed': { machine: 'participant' }, 'raise.failed': { machine: 'instance' }, 'raise.succeeded': { machine: 'instance' } },
  metadata: { instance: { name: { writer: 'creator', filterable: true }, tagline: { writer: 'creator', filterable: false }, ticker: { writer: 'creator', filterable: true } } },
  stacks: STACKS,
  programs: PROGRAMS,
};

export type TokenRaiseWebhookPayload = ProtocolWebhookPayload<TokenRaiseProtocolTypes>;
/** Verify a signed webhook delivery for this protocol's events. */
export function verifyTokenRaiseWebhook(secret: string, body: string | Uint8Array, headers: WebhookHeaders, options?: VerifyWebhookOptions): VerifiedWebhook<TokenRaiseWebhookPayload> {
  return verifyProtocolWebhook<TokenRaiseWebhookPayload>(secret, body, headers, options);
}
export { verifyWebhook } from '../index';
export default TOKEN_RAISE_PROTOCOL;

// --- assertions --------------------------------------------------------------

type Equal<TLeft, TRight> =
  (<T>() => T extends TLeft ? 1 : 2) extends
  (<T>() => T extends TRight ? 1 : 2)
    ? true
    : false;
type Assert<T extends true> = T;

// Any generated definition is a `ProtocolDefinition`.
const asAny: ProtocolDefinition = TOKEN_RAISE_PROTOCOL;
void asAny;

const publishableClient = createProtocolClient(TOKEN_RAISE_PROTOCOL, {
  auth: { publishableKey: 'a4_pk_test' },
});
const secretClient = createProtocolClient(TOKEN_RAISE_PROTOCOL, {
  auth: { secretKey: 'a4_sk_test' },
});
const tokenClient = createProtocolClient(TOKEN_RAISE_PROTOCOL, { auth: { token: 'jwt' } });
const declaredSecretClient = createProtocolClient(TOKEN_RAISE_PROTOCOL, {
  auth: { getToken: async () => 'jwt' },
  keyClass: 'secret',
});
const defaultClient = createProtocolClient(TOKEN_RAISE_PROTOCOL);

export type KeyClassAssertions = [
  Assert<Equal<typeof publishableClient.keyClass, 'publishable'>>,
  Assert<Equal<typeof secretClient.keyClass, 'secret'>>,
  Assert<Equal<typeof tokenClient.keyClass, 'publishable'>>,
  Assert<Equal<typeof declaredSecretClient.keyClass, 'secret'>>,
  Assert<Equal<typeof defaultClient.keyClass, 'publishable'>>,
  Assert<Equal<keyof typeof publishableClient.steps, 'contribute' | 'claim' | 'refund'>>,
  Assert<Equal<keyof typeof secretClient.steps, 'create' | 'contribute' | 'claim' | 'refund'>>,
];

const WALLET = 'So11111111111111111111111111111111111111112';

export async function publishableUsage(): Promise<string> {
  // @ts-expect-error `create` is a secret step: absent from a publishable client.
  await publishableClient.steps.create.prepare({ wallet: WALLET, deadline: '1', hardCap: '2', members: [], quoteMint: WALLET, softCap: '1', walletCap: '1' });
  // @ts-expect-error a token client is publishable unless declared secret.
  await tokenClient.steps.create.prepare({ wallet: WALLET, deadline: '1', hardCap: '2', members: [], quoteMint: WALLET, softCap: '1', walletCap: '1' });

  const result = await publishableClient.steps.contribute.prepare(
    { instance: 'instance-1', wallet: WALLET, amount: 5n },
    { transactionFormat: 'v0' }
  );
  await publishableClient.steps.contribute.prepare({ instance: 'instance-1', wallet: WALLET, amount: '5' });
  // @ts-expect-error u64 arguments take a decimal string or bigint, not a number.
  await publishableClient.steps.contribute.prepare({ instance: 'instance-1', wallet: WALLET, amount: 5 });
  // @ts-expect-error instance and participant steps need the instance id.
  await publishableClient.steps.contribute.prepare({ wallet: WALLET, amount: '5' });
  // @ts-expect-error the arguments are required.
  await publishableClient.steps.contribute.prepare({ instance: 'instance-1', wallet: WALLET });

  if (result.ok) {
    const executionId: string = result.intent.executionId;
    const base64: string | undefined = result.transactions[0]?.transactionBase64;
    await publishableClient.submit(result.intent, [base64 ?? '', new Uint8Array(64)]);
    return executionId;
  }
  // Exhaustive over the step's guard codes plus the access and state codes.
  const code = result.rejection.rejected;
  switch (code) {
    case 'raise_not_open':
    case 'deadline_passed':
    case 'wallet_cap_exceeded':
    case 'hard_cap_exceeded':
      return `guard:${code}`;
    case 'step_requires_secret_key':
    case 'step_not_in_session_scope':
    case 'step_not_client_callable':
      return `access:${code}`;
    case 'instance_terminal':
    case 'participant_terminal':
      return `state:${code}`;
    default: {
      const unreachable: never = code;
      return unreachable;
    }
  }
}

export type RejectionAssertions = [
  Assert<Equal<
    Extract<Awaited<ReturnType<typeof publishableClient.steps.claim.prepare>>, { ok: false }>['rejection']['rejected'],
    'raise_not_finalized' | ProtocolAccessRejectionCode | ProtocolStateRejectionCode
  >>,
];

export async function secretUsage(): Promise<void> {
  const created = await secretClient.steps.create.prepare({
    wallet: WALLET,
    deadline: 1_900_000_000n,
    hardCap: '1000',
    members: [WALLET],
    quoteMint: WALLET,
    softCap: '500',
    walletCap: '100',
  });
  if (!created.ok) {
    const code: 'soft_cap_above_hard_cap' | ProtocolAccessRejectionCode | ProtocolStateRejectionCode =
      created.rejection.rejected;
    void code;
  }
  await declaredSecretClient.steps.create.prepare({
    wallet: WALLET,
    deadline: '1',
    hardCap: '2',
    members: [],
    quoteMint: WALLET,
    softCap: '1',
    walletCap: '1',
  });
  // Create steps take no instance.
  // @ts-expect-error `instance` is not an argument of a create step.
  await secretClient.steps.create.prepare({ instance: 'x', wallet: WALLET, deadline: '1', hardCap: '2', members: [], quoteMint: WALLET, softCap: '1', walletCap: '1' });
}

export async function readUsage(): Promise<void> {
  const page = await publishableClient.instances.list({ state: 'open', metadata: { ticker: 'ABC' } });
  const state: TokenRaiseInstanceState | undefined = page.instances[0]?.state;
  void state;
  // @ts-expect-error not an instance state.
  await publishableClient.instances.list({ state: 'pending' });

  const detail = await publishableClient.instances.get('instance-1', { wallet: WALLET });
  const hardCap: ProtocolInteger = detail.params.hardCap;
  const name: string | undefined = detail.metadata.name;
  void hardCap;
  void name;

  const allowed = await publishableClient.allowedSteps({ instance: 'instance-1', wallet: WALLET });
  const first = allowed[0];
  if (first && first.step === 'contribute' && first.reason === 'raise_not_open') void first;

  const actions = await publishableClient.actions({ wallet: WALLET, limit: 20 });
  const createStep = actions.createSteps[0];
  if (createStep && createStep.step === 'create' && createStep.reason === 'soft_cap_above_hard_cap') {
    const argNames: string[] | undefined = createStep.args?.map((arg) => arg.name);
    void argNames;
  }
  for (const related of actions.instances) {
    const relatedState: TokenRaiseInstanceState = related.state;
    const relatedParticipant: TokenRaiseParticipantState | null = related.participantState;
    const isAdmin: boolean = related.relation.includes('role:admin');
    void relatedState;
    void relatedParticipant;
    void isAdmin;
    const step = related.steps[0];
    if (step && step.step === 'contribute' && step.scope === 'participant' && step.reason === 'raise_not_open') void step;
  }
  const nextCursor: string | null = actions.nextCursor;
  if (nextCursor) await publishableClient.actions({ wallet: WALLET, cursor: nextCursor, instance: 'instance-1' });
  // @ts-expect-error the wallet is required.
  await publishableClient.actions({ instance: 'instance-1' });
  // @ts-expect-error not a relation.
  const badRelation: (typeof actions.instances)[number]['relation'][number] = 'owner';
  void badRelation;

  const participant = await publishableClient.participants.get({ instance: 'instance-1', wallet: WALLET });
  const participantState: TokenRaiseParticipantState | null = participant.state;
  void participantState;

  await secretClient.metadata.instance('instance-1').update({ tagline: null, name: 'Raise' });
  // @ts-expect-error not a metadata field.
  await secretClient.metadata.instance('instance-1').update({ unknown: 'x' });

  const subscription = publishableClient.events('instance-1', {
    onEvent(event) {
      if (event.type === 'emit') {
        const name: TokenRaiseEvent = event.event;
        void name;
      }
    },
  });
  subscription.close();

  const handle = publishableClient.instances.use('instance-1');
  const snapshotState: TokenRaiseInstanceState | undefined = handle.get()?.instance.state;
  void snapshotState;

  const stack: typeof METEORA_PRESALE_STACK = publishableClient.stacks.meteoraPresale;
  void stack;
}

export function webhookUsage(body: string, headers: WebhookHeaders): string {
  const { payload } = verifyTokenRaiseWebhook('whsec', body, headers);
  if (payload.event === 'contribution.confirmed') {
    const state: TokenRaiseParticipantState = payload.state;
    return state;
  }
  const state: TokenRaiseInstanceState = payload.state;
  return state;
}

// --- sessions ----------------------------------------------------------------

export async function sessionUsage(): Promise<void> {
  const publishableSession = await createSession(
    { protocols: { tokenRaise: TOKEN_RAISE_PROTOCOL } },
    { auth: { publishableKey: 'a4_pk_test' } }
  );
  const secretSession = await createSession(
    { protocols: { tokenRaise: TOKEN_RAISE_PROTOCOL } },
    { auth: { secretKey: 'a4_sk_test' } }
  );
  const declaredSession = await createSession(
    { protocols: { tokenRaise: TOKEN_RAISE_PROTOCOL } },
    { auth: { getToken: async () => 'jwt' }, protocolKeyClass: 'secret' }
  );

  type SessionAssertions = [
    Assert<Equal<typeof publishableSession.protocols.tokenRaise.keyClass, 'publishable'>>,
    Assert<Equal<typeof secretSession.protocols.tokenRaise.keyClass, 'secret'>>,
    Assert<Equal<typeof declaredSession.protocols.tokenRaise.keyClass, 'secret'>>,
    Assert<Equal<keyof typeof publishableSession.protocols.tokenRaise.steps, 'contribute' | 'claim' | 'refund'>>,
    // Hoisted stacks and programs are promoted into the session.
    Assert<Equal<
      typeof secretSession.stacks.meteoraPresale,
      ConnectedArete<typeof METEORA_PRESALE_STACK, typeof METEORA_PRESALE_STACK>
    >>,
    Assert<Equal<typeof secretSession.programs.squadsV4, ProgramInterface<typeof SQUADS_V4_PROGRAM>>>,
  ];
  const assertions: SessionAssertions | undefined = undefined;
  void assertions;

  // @ts-expect-error a publishable session has no secret steps.
  void publishableSession.protocols.tokenRaise.steps.create;
  await secretSession.protocols.tokenRaise.steps.create.prepare({
    wallet: WALLET,
    deadline: '1',
    hardCap: '2',
    members: [],
    quoteMint: WALLET,
    softCap: '1',
    walletCap: '1',
  });
  // The protocol's members are the session's connected members.
  const connected: typeof secretSession.stacks.meteoraPresale =
    secretSession.protocols.tokenRaise.stacks.meteoraPresale;
  void connected;
  const squads: typeof secretSession.programs.squadsV4 =
    secretSession.protocols.tokenRaise.programs.squadsV4;
  void squads;

  // Sessions without protocols keep their types and get an empty map.
  const plain = await createSession({ stacks: { presale: METEORA_PRESALE_STACK } });
  type PlainAssertions = [
    Assert<Equal<keyof typeof plain.protocols, never>>,
    Assert<Equal<keyof typeof plain.stacks, 'presale'>>,
  ];
  const plainAssertions: PlainAssertions | undefined = undefined;
  void plainAssertions;
}
