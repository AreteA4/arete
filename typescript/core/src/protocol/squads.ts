/**
 * Account Squads: the Squads v4 multisigs an Arete account has attached.
 *
 * Admin steps of a protocol are proposed to a multisig attached to the
 * deployment owner's account, by a proposer key that belongs to the account
 * and may only initiate. The account attaches each multisig once (or creates
 * one) and adds the proposer as an Initiate-only member; production
 * deployments refuse a Squads step whose multisig is not an `active`
 * attachment (`protocol-squads-setup-required`).
 *
 * The easiest path is the dashboard (`SQUADS_DASHBOARD_URL`), which signs
 * with any standard browser wallet, hardware wallets included. This client
 * is for scripts and hot wallets: every write returns an unsigned
 * transaction for one wallet to sign, and `submit` sends and confirms it.
 * Arete never holds or asks for a member key.
 */
import { classifyApiKey, isBrowserEnvironment, readEnvironmentVariable, resolveAuthConfig } from '../auth-keys';
import { AreteError, type AuthConfig } from '../types';
import {
  ARETE_API_URL_ENV,
  bytesToBase64,
  isRecord,
  jsonBody,
  readJson,
  resolveFetch,
  responseError,
  segment,
  trimSlash,
} from './client';
import type { ProtocolTransactionFormat } from './types';

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/** The dashboard page that sets up and manages the account's multisigs. */
export const SQUADS_DASHBOARD_URL = 'https://app.arete.run/dashboard/squads';

/** The wire code of a deployment refused because its multisig is not set up. */
export const SQUADS_SETUP_REQUIRED_CODE = 'protocol-squads-setup-required' as const;

const DEFAULT_API_URL = 'https://api.arete.run/api';

/** The dashboard page for the account's multisigs, or for one of them. */
export function squadsDashboardUrl(multisig?: string): string {
  const trimmed = multisig?.trim();
  return trimmed ? `${SQUADS_DASHBOARD_URL}/${encodeURIComponent(trimmed)}` : SQUADS_DASHBOARD_URL;
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** `notStarted`: nothing attached; `ready`: one is `active`; `needsAttention`: none is. */
export type SquadsSetup = 'notStarted' | 'ready' | 'needsAttention';

/** `active` while the proposer is an Initiate-only member. */
export type SquadsAttachmentState = 'pending' | 'active' | 'archived';

/** Why an attachment is not `active`. */
export type SquadsAttachmentProblem =
  | 'not_member'
  | 'permissions_invalid'
  | 'squads_multisig_missing'
  | 'squads_multisig_invalid'
  | 'create_unconfirmed';

/** How the multisig was attached: by address, created through Arete, or by a deployment. */
export type SquadsAttachmentOrigin = 'page' | 'created' | 'deployment';

/** A Squads member permission. */
export type SquadsPermission = 'initiate' | 'vote' | 'execute';

/** The account's proposer. It proposes; it never votes or executes. */
export interface SquadsAccountProposer {
  readonly pubkey: string;
  /** Always `initiate`. */
  readonly permissions: 'initiate';
  /** The proposer's balance, for information; `null` when the chain did not answer. */
  readonly lamports: number | null;
}

export interface SquadsMember {
  readonly pubkey: string;
  /** The Squads permission mask (Initiate 1, Vote 2, Execute 4). */
  readonly mask: number;
  readonly permissions: readonly SquadsPermission[];
}

/** A production deployment whose Squads steps propose to the multisig (as the deploy gate last resolved them). */
export interface SquadsAttachmentDeployment {
  readonly id: string;
  readonly protocolId: string;
  readonly protocolName: string;
  readonly protocolSlug: string;
  readonly status: string;
  /** The Squads steps that propose to it. */
  readonly steps: readonly string[];
}

/**
 * An `archived` attachment that was `active` (the proposer was removed or
 * its permissions changed), with the deployments that used it then. It goes
 * away once the attachment is active again or forgotten.
 */
export interface SquadsNotification {
  readonly multisig: string;
  readonly problem: SquadsAttachmentProblem | (string & {}) | null;
  readonly problemDescription: string | null;
  readonly deployments: readonly {
    readonly id: string;
    readonly protocolName?: string;
    readonly steps: readonly string[];
  }[];
  /** RFC 3339. */
  readonly since: string;
  readonly dashboardUrl: string;
}

/** A multisig attached to the account, refreshed from the chain. */
export interface SquadsAttachment {
  readonly address: string;
  readonly label: string | null;
  readonly state: SquadsAttachmentState;
  readonly problem: SquadsAttachmentProblem | (string & {}) | null;
  /** The problem as a plain sentence. */
  readonly problemDescription: string | null;
  readonly createdVia: SquadsAttachmentOrigin;
  /** RFC 3339. */
  readonly stateChangedAt: string;
  /** RFC 3339. */
  readonly createdAt: string;
  /** `null` when the account could not be read. */
  readonly threshold: number | null;
  readonly members: readonly SquadsMember[] | null;
  /** The proposer's mask, when it is a member. */
  readonly proposerMask: number | null;
  /** Vault 0. */
  readonly vault: string;
  /** The multisig in the Squads app. */
  readonly squadsUrl: string;
  /** The multisig's dashboard page. */
  readonly dashboardUrl: string;
  /** The deployment-config snippet that names this multisig. */
  readonly deploymentConfigSnippet: string;
  readonly deployments: readonly SquadsAttachmentDeployment[];
}

/** A config proposal Arete built for the multisig (adding or removing the proposer). */
export interface SquadsConfigProposal {
  readonly kind: 'addProposer' | 'removeProposer';
  readonly transactionIndex: number;
  readonly proposal: string;
  readonly status:
    | 'draft'
    | 'active'
    | 'approved'
    | 'executed'
    | 'rejected'
    | 'cancelled'
    | 'executing'
    | 'missing';
  readonly approvals: readonly string[];
  readonly threshold: number | null;
  readonly squadsUrl: string;
}

/** A proposal the account's proposer made for a deployment's admin step. */
export interface SquadsStepProposal {
  readonly deploymentId: string;
  readonly step: string;
  readonly instanceId: string | null;
  readonly status: string;
  readonly transactionIndex: number;
  readonly proposal: string;
  readonly createdAt: string;
  readonly squadsUrl: string;
}

export interface SquadsAttachmentDetail extends SquadsAttachment {
  /** The account's proposer pubkey. */
  readonly proposer: string;
  readonly configProposals: readonly SquadsConfigProposal[];
  readonly proposals: readonly SquadsStepProposal[];
}

/** `GET /account/squads`. */
export interface SquadsAccountStatus {
  readonly setup: SquadsSetup;
  readonly proposer: SquadsAccountProposer;
  readonly dashboardUrl: string;
  readonly multisigs: readonly SquadsAttachment[];
}

export type SquadsPreparedTransactionKind =
  | 'createMultisig'
  | 'addProposer'
  | 'approve'
  | 'execute'
  | 'removeProposer';

/**
 * An unsigned transaction for one wallet (`signer`, which also pays) to
 * sign. Pass the signed bytes and `transactionId` to `submit`.
 */
export interface SquadsPreparedTransaction {
  readonly transactionId: string;
  readonly kind: SquadsPreparedTransactionKind;
  /** Wire bytes; the signer's signature slot is zeroed. */
  readonly transactionBase64: string;
  readonly transactionFormat: ProtocolTransactionFormat;
  readonly signer: string;
  readonly lastValidBlockHeight: number;
  /** The multisig (for `createMultisig`, the new address). */
  readonly multisig: string;
  readonly transactionIndex: number | null;
  readonly proposal: string | null;
  /** What to do once it confirms. */
  readonly next: string;
}

/** `pending`: sent, not confirmed yet; submit the same body again. `expired`: prepare a new one. */
export type SquadsSubmitStatus = 'confirmed' | 'failed' | 'pending' | 'expired';

export interface SquadsSubmitResult {
  readonly transactionId: string;
  readonly signature: string;
  readonly status: SquadsSubmitStatus;
  readonly error: string | null;
  readonly multisig: SquadsAttachment;
}

interface SignalOption {
  readonly signal?: AbortSignal;
}

export interface SquadsAttachInput extends SignalOption {
  /** The Squads v4 multisig account (not one of its vaults). */
  readonly address: string;
  /** 1 to 64 characters. */
  readonly label?: string;
}

export interface SquadsCreateMember {
  readonly pubkey: string;
  readonly permissions: readonly SquadsPermission[];
}

export interface SquadsCreateInput extends SignalOption {
  /** The members; do not list the proposer, which is added Initiate-only. */
  readonly members: readonly SquadsCreateMember[];
  /** Between 1 and the number of voting members. */
  readonly threshold: number;
  /** A member who signs and pays the creation fee and rent. */
  readonly creator: string;
  readonly label?: string;
  readonly transactionFormat?: ProtocolTransactionFormat;
}

export interface SquadsMemberInput extends SignalOption {
  /** The member who signs and pays. */
  readonly member: string;
  /** `v0` (the default, which browser wallets sign) or `v1`. */
  readonly transactionFormat?: ProtocolTransactionFormat;
}

export interface SquadsSubmitInput extends SignalOption {
  readonly transactionId: string;
  /** The signed transaction: base64 wire bytes or the bytes themselves. */
  readonly signedTransaction: string | Uint8Array;
}

/** `details` of a `protocol-squads-setup-required` refusal. */
export interface SquadsSetupRequiredDetails {
  readonly step: string;
  readonly multisig: string;
  readonly state: SquadsAttachmentState;
  readonly problem: SquadsAttachmentProblem | (string & {}) | null;
  readonly proposer: string;
  /** The multisig's dashboard page, where the proposer is added. */
  readonly setupUrl: string;
  readonly commands?: { readonly status?: string; readonly addProposer?: string };
}

export interface SquadsSetupRequired {
  readonly message: string;
  readonly details: SquadsSetupRequiredDetails;
}

export interface SquadsClientOptions {
  /**
   * A secret or agent key (`auth.secretKey`) or a dashboard token
   * (`auth.token` / `auth.getToken`). Outside browsers it defaults to
   * `ARETE_API_KEY`, then the `a4` login. Publishable keys are refused.
   */
  auth?: AuthConfig;
  /** `<origin>/api`. Defaults to `ARETE_API_URL` (an origin) outside browsers, then `https://api.arete.run/api`. */
  apiUrl?: string;
  fetch?: typeof fetch;
}

/** The account's Squads multisigs, over `<api>/account/squads`. */
export interface SquadsClient {
  /** The setup state, the proposer and every attachment (refreshed). */
  status(options?: SignalOption): Promise<SquadsAccountStatus>;
  /** Attachments archived after being active, with the deployments that used them (no chain read). */
  notifications(options?: SignalOption): Promise<SquadsNotification[]>;
  /** One attachment with its config proposals and Arete's proposals (refreshed). */
  get(multisig: string, options?: SignalOption): Promise<SquadsAttachmentDetail>;
  /** Attach an existing multisig: `active` if the proposer is already an Initiate-only member, else `pending`. */
  attach(input: SquadsAttachInput): Promise<SquadsAttachment>;
  /** An unsigned `multisigCreateV2` with the proposer already an Initiate-only member, for `creator` to sign. */
  create(input: SquadsCreateInput): Promise<SquadsPreparedTransaction>;
  /** An unsigned config proposal adding the proposer (Initiate only), for a member who can initiate. */
  addProposer(multisig: string, input: SquadsMemberInput): Promise<SquadsPreparedTransaction>;
  /** An unsigned approval of config proposal `transactionIndex`, for a member who can vote. */
  approve(multisig: string, transactionIndex: number, input: SquadsMemberInput): Promise<SquadsPreparedTransaction>;
  /** An unsigned execution of approved config proposal `transactionIndex`, for a member who can execute. */
  execute(multisig: string, transactionIndex: number, input: SquadsMemberInput): Promise<SquadsPreparedTransaction>;
  /** An unsigned config proposal removing the proposer; once executed the attachment is `archived`. */
  removeProposer(multisig: string, input: SquadsMemberInput): Promise<SquadsPreparedTransaction>;
  /** Forget a `pending` or `archived` attachment (an `active` one is refused). */
  forget(multisig: string, options?: SignalOption): Promise<void>;
  /** Send a signed prepared transaction and wait (about 30 seconds) for it to confirm. */
  submit(input: SquadsSubmitInput): Promise<SquadsSubmitResult>;
}

// ---------------------------------------------------------------------------
// The 409 deploy gate
// ---------------------------------------------------------------------------

function setupRequiredFromBody(body: unknown, fallbackMessage?: string): SquadsSetupRequired | undefined {
  if (!isRecord(body)) return undefined;
  const code = typeof body['code'] === 'string' ? body['code'].trim().toLowerCase().replace(/_/g, '-') : undefined;
  if (code !== SQUADS_SETUP_REQUIRED_CODE || !isRecord(body['details'])) return undefined;
  const message = typeof body['message'] === 'string'
    ? body['message']
    : typeof body['error'] === 'string'
      ? body['error']
      : fallbackMessage ?? 'The Squads multisig is not set up for your account';
  return { message, details: body['details'] as unknown as SquadsSetupRequiredDetails };
}

/**
 * The step, multisig and dashboard link of a `protocol-squads-setup-required`
 * refusal (deployment create or activation), from an `AreteError` or a raw
 * response body; `undefined` for anything else.
 */
export function squadsSetupRequired(error: unknown): SquadsSetupRequired | undefined {
  if (error instanceof AreteError) {
    const details = isRecord(error.details) ? error.details : undefined;
    const body = details?.['body'];
    const found = setupRequiredFromBody(body, error.message);
    if (found) return found;
    const wire = typeof details?.['wireErrorCode'] === 'string' ? details['wireErrorCode'] : error.code;
    if (isRecord(body) && typeof wire === 'string'
      && wire.trim().toLowerCase().replace(/_/g, '-') === SQUADS_SETUP_REQUIRED_CODE) {
      return setupRequiredFromBody({ ...body, code: SQUADS_SETUP_REQUIRED_CODE }, error.message);
    }
    return undefined;
  }
  return setupRequiredFromBody(error);
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

function resolveApiUrl(apiUrl: string | undefined): string {
  const configured = apiUrl?.trim();
  if (configured) return trimSlash(configured);
  const origin = isBrowserEnvironment() ? undefined : readEnvironmentVariable(ARETE_API_URL_ENV)?.trim();
  if (origin) return `${trimSlash(origin).replace(/\/api$/, '')}/api`;
  return DEFAULT_API_URL;
}

type BearerSource = () => Promise<string>;

function resolveBearer(configured: AuthConfig | undefined): BearerSource {
  const auth = resolveAuthConfig(configured);
  const secretOnly = 'Account Squads routes need a secret or agent key (auth.secretKey, ARETE_API_KEY or the a4 login) or a dashboard token';
  if (auth?.token !== undefined) {
    if (!auth.token) throw new AreteError('Authentication token is empty', 'TOKEN_INVALID');
    const token = auth.token;
    return async () => token;
  }
  if (auth?.getToken !== undefined) {
    const getToken = auth.getToken;
    return async () => {
      const result = await getToken();
      const token = typeof result === 'string' ? result : result?.token;
      if (!token) throw new AreteError('getToken returned an empty token', 'TOKEN_INVALID');
      return token;
    };
  }
  const key = auth?.secretKey
    ?? (auth?.publishableKey !== undefined && classifyApiKey(auth.publishableKey) === 'secret'
      ? auth.publishableKey
      : undefined);
  if (key === undefined) {
    if (auth?.publishableKey !== undefined) {
      throw new AreteError(`${secretOnly}; publishable keys are refused. Use the dashboard (${SQUADS_DASHBOARD_URL}) from a browser.`, 'INVALID_CONFIG');
    }
    throw new AreteError(`${secretOnly}; none was configured.`, 'INVALID_CONFIG');
  }
  return async () => key;
}

function memberBody(input: SquadsMemberInput, what: string): Record<string, unknown> {
  if (!isRecord(input) || typeof input.member !== 'string' || input.member.trim().length === 0) {
    throw new AreteError(`${what} requires a member`, 'INVALID_CONFIG');
  }
  return {
    member: input.member.trim(),
    ...(input.transactionFormat ? { transactionFormat: input.transactionFormat } : {}),
  };
}

function proposalIndex(value: number): string {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new AreteError('transactionIndex must be a non-negative integer', 'INVALID_CONFIG');
  }
  return String(value);
}

/**
 * A client for the calling account's Squads multisigs. Authenticate with a
 * secret or agent key (never a publishable key) or a dashboard token.
 *
 * ```ts
 * const squads = createSquadsClient({ auth: { secretKey: process.env.ARETE_API_KEY } });
 * const { setup, dashboardUrl } = await squads.status();
 * ```
 */
export function createSquadsClient(options?: SquadsClientOptions): SquadsClient {
  const fetchImpl = resolveFetch(options?.fetch);
  const bearer = resolveBearer(options?.auth);
  const base = `${resolveApiUrl(options?.apiUrl)}/account/squads`;
  const multisigUrl = (multisig: string) => `${base}/multisigs/${segment(typeof multisig === 'string' ? multisig.trim() : multisig, 'multisig')}`;

  const call = async (
    what: string,
    method: string,
    url: string,
    body: unknown,
    signal: AbortSignal | undefined
  ): Promise<unknown> => {
    const headers = new Headers({ Authorization: `Bearer ${await bearer()}` });
    if (body !== undefined) headers.set('Content-Type', 'application/json');
    const response = await fetchImpl(url, {
      method,
      headers,
      body: body === undefined ? undefined : jsonBody(body),
      signal,
    });
    const parsed = await readJson(response);
    if (!response.ok) throw responseError(what, response, parsed);
    return parsed;
  };

  return {
    async status(statusOptions) {
      return await call('Squads status', 'GET', base, undefined, statusOptions?.signal) as SquadsAccountStatus;
    },
    async notifications(notificationOptions) {
      return await call('Squads notifications', 'GET', `${base}/notifications`, undefined, notificationOptions?.signal) as SquadsNotification[];
    },
    async get(multisig, getOptions) {
      return await call('Squads multisig read', 'GET', multisigUrl(multisig), undefined, getOptions?.signal) as SquadsAttachmentDetail;
    },
    async attach(input) {
      if (!isRecord(input) || typeof input.address !== 'string' || input.address.trim().length === 0) {
        throw new AreteError('attach requires an address', 'INVALID_CONFIG');
      }
      return await call('Squads attach', 'POST', `${base}/multisigs`, {
        address: input.address.trim(),
        ...(input.label !== undefined ? { label: input.label } : {}),
      }, input.signal) as SquadsAttachment;
    },
    async create(input) {
      if (!isRecord(input) || !Array.isArray(input.members) || typeof input.creator !== 'string'
        || typeof input.threshold !== 'number') {
        throw new AreteError('create requires members, a threshold and a creator', 'INVALID_CONFIG');
      }
      return await call('Squads create', 'POST', `${base}/multisigs/create`, {
        members: input.members.map((member) => ({ pubkey: member.pubkey, permissions: [...member.permissions] })),
        threshold: input.threshold,
        creator: input.creator,
        ...(input.label !== undefined ? { label: input.label } : {}),
        ...(input.transactionFormat ? { transactionFormat: input.transactionFormat } : {}),
      }, input.signal) as SquadsPreparedTransaction;
    },
    async addProposer(multisig, input) {
      const body = memberBody(input, 'addProposer');
      return await call('Squads add-proposer', 'POST', `${multisigUrl(multisig)}/add-proposer`, body, input.signal) as SquadsPreparedTransaction;
    },
    async approve(multisig, transactionIndex, input) {
      const body = memberBody(input, 'approve');
      return await call(
        'Squads approve',
        'POST',
        `${multisigUrl(multisig)}/proposals/${proposalIndex(transactionIndex)}/approve`,
        body,
        input.signal
      ) as SquadsPreparedTransaction;
    },
    async execute(multisig, transactionIndex, input) {
      const body = memberBody(input, 'execute');
      return await call(
        'Squads execute',
        'POST',
        `${multisigUrl(multisig)}/proposals/${proposalIndex(transactionIndex)}/execute`,
        body,
        input.signal
      ) as SquadsPreparedTransaction;
    },
    async removeProposer(multisig, input) {
      const body = memberBody(input, 'removeProposer');
      return await call('Squads remove-proposer', 'POST', `${multisigUrl(multisig)}/remove-proposer`, body, input.signal) as SquadsPreparedTransaction;
    },
    async forget(multisig, forgetOptions) {
      await call('Squads forget', 'DELETE', multisigUrl(multisig), undefined, forgetOptions?.signal);
    },
    async submit(input) {
      if (!isRecord(input) || typeof input.transactionId !== 'string' || input.transactionId.length === 0) {
        throw new AreteError('submit requires a transactionId', 'INVALID_CONFIG');
      }
      const signed = input.signedTransaction instanceof Uint8Array
        ? bytesToBase64(input.signedTransaction)
        : input.signedTransaction;
      if (typeof signed !== 'string' || signed.length === 0) {
        throw new AreteError('submit requires the signed transaction', 'INVALID_CONFIG');
      }
      return await call('Squads submit', 'POST', `${base}/transactions`, {
        transactionId: input.transactionId,
        signedTransactionBase64: signed,
      }, input.signal) as SquadsSubmitResult;
    },
  };
}
