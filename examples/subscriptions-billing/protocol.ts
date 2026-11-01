/**
 * `subscriptionsBilling`: recurring token billing on the Solana Foundation
 * `subscriptions` program, built from instruction templates only.
 *
 * One protocol instance is one on-chain plan. `createPlan` is the only create
 * step and binds the plan PDA from the merchant wallet and `planId`, so a second
 * create for the same pair fails on chain. Plans created outside Arete can't be
 * adopted.
 *
 * `planId`, `mint`, `amount` and `periodHours` are params, so they are immutable
 * for the life of the instance, as the program keeps them on the plan.
 * `updatePlan` can change only the status, the end time and the metadata.
 *
 * Roles: `merchant` (the creating wallet) and `puller` (`pullers[0]`, or the
 * default pubkey when the plan has none, which leaves collecting to the
 * merchant). Subscribers are participants, not a role. The builder holds the
 * puller's key; Arete holds no merchant key.
 *
 * Templates pass each instruction argument as one expression, and expressions
 * can't build a struct. The program's struct arguments therefore come from the
 * step's args, and guards pin every field that must match the params or the
 * chain. See "Client-supplied structs" in README.md.
 */
import subscriptions from "@programs/subscriptions";
import { defineProtocol, ix } from "@usearete/protocol";

const DEFAULT_PUBKEY = '"11111111111111111111111111111111"';
const TOKEN_PROGRAM = '"TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"';

const pubkeys = { type: "array", items: { type: "string", format: "pubkey" }, maxItems: 4 } as const;
const metadataUri = {
  type: "array",
  items: { type: "integer", minimum: 0, maximum: 255 },
  maxItems: 128,
  description: "The metadata URI as UTF-8 bytes, zero-padded to exactly 128 items.",
} as const;

/** A struct argument: every field required, nothing else allowed. */
function struct(properties: Record<string, unknown>) {
  return { type: "object", properties, required: Object.keys(properties), additionalProperties: false } as never;
}

const collectArgs = {
  subscriber: "pubkey",
  destination: "pubkey",
  transferData: struct({
    amount: { type: "string", format: "u64" },
    delegator: { type: "string", format: "pubkey" },
    mint: { type: "string", format: "pubkey" },
  }),
} as const;

const collectGuards = {
  destination_not_allowed: `args.destination != ${DEFAULT_PUBKEY} && args.destination in reads.plan.data.destinations`,
  transfer_mismatch:
    "args.transferData.delegator == args.subscriber && args.transferData.mint == params.mint && args.transferData.amount == params.amount",
} as const;

const collectBuild = [
  ix(subscriptions, "transferSubscription", {
    args: { transferData: "args.transferData" },
    accounts: {
      subscriptionPda: 'pda("subscriptions", "subscriptionDelegation", instance.accounts.plan, args.subscriber)',
      planPda: "instance.accounts.plan",
      subscriptionAuthority: 'pda("subscriptions", "subscriptionAuthority", args.subscriber, params.mint)',
      delegatorAta: "ata(args.subscriber, params.mint)",
      receiverAta: "ata(args.destination, params.mint)",
      caller: "wallet",
      tokenMint: "params.mint",
      tokenProgram: TOKEN_PROGRAM,
    },
  }),
];

export default defineProtocol({
  name: "subscriptionsBilling",
  version: "0.1.0",
  description: "One subscription plan on the Solana Foundation subscriptions program: subscribe, cancel, resume and collect.",
  programs: { subscriptions },
  params: { planId: "u64", mint: "pubkey", amount: "u64", periodHours: "u64" },
  roles: {
    merchant: { description: "Owns the plan: updates it and collects." },
    puller: { description: "The one wallet the merchant lets collect; the default pubkey when there is none." },
  },
  instanceVisibility: "account",
  reads: {
    plan: { kind: "account", program: "subscriptions", account: "plan", address: "instance.accounts.plan" },
    subscriptionAuthority: {
      kind: "account",
      program: "subscriptions",
      account: "subscriptionAuthority",
      address: 'pda("subscriptions", "subscriptionAuthority", wallet, params.mint)',
      optional: true,
    },
    subscription: {
      kind: "account",
      program: "subscriptions",
      account: "subscriptionDelegation",
      address: 'pda("subscriptions", "subscriptionDelegation", instance.accounts.plan, wallet)',
      optional: true,
    },
  },
  machines: {
    instance: {
      initial: "open",
      states: {
        open: { kind: "recorded", description: "Created; the plan has not been read yet." },
        active: { kind: "derived", description: "The plan accepts new subscribers." },
        // Not terminal: a terminal instance would reject cancel. The program
        // never takes a plan out of sunset; the exit keeps the machine honest
        // about whatever status the chain reports.
        sunset: { kind: "derived", description: "No new subscribers; existing ones run until the end time." },
      },
      derive: [
        { from: ["open", "sunset"], to: "active", when: "reads.plan.status == 1" },
        { from: ["open", "active"], to: "sunset", when: "reads.plan.status == 0" },
      ],
      reads: ["plan"],
    },
    participant: {
      initial: "prospect",
      states: {
        prospect: "recorded",
        subscribed: "recorded",
        // Not terminal: `resume` moves back to `subscribed`.
        canceled: "recorded",
      },
    },
  },
  steps: {
    createPlan: {
      scope: "create",
      title: "Create the plan",
      authority: "creator",
      access: "secret",
      args: {
        planId: "u64",
        mint: "pubkey",
        amount: "u64",
        periodHours: "u64",
        planData: struct({
          planId: { type: "string", format: "u64" },
          mint: { type: "string", format: "pubkey" },
          terms: struct({
            amount: { type: "string", format: "u64" },
            periodHours: { type: "string", format: "u64" },
            createdAt: { type: "string", format: "i64", description: "Ignored: the program sets it. Pass 0." },
          }),
          endTs: { type: "string", format: "i64" },
          destinations: pubkeys,
          pullers: pubkeys,
          metadataUri,
        }),
      },
      guards: {
        plan_data_mismatch:
          "args.planData.planId == args.planId && args.planData.mint == args.mint && args.planData.terms.amount == args.amount && args.planData.terms.periodHours == args.periodHours",
        amount_zero: "args.amount > 0",
        period_out_of_range: "args.periodHours > 0 && args.periodHours <= 8760",
        arrays_not_padded:
          "len(args.planData.destinations) == 4 && len(args.planData.pullers) == 4 && len(args.planData.metadataUri) == 128",
        // `collect` checks the destination against this list, so a plan with
        // none could never be collected through this protocol.
        no_destination: `args.planData.destinations[0] != ${DEFAULT_PUBKEY}`,
        too_many_pullers: `args.planData.pullers[1] == ${DEFAULT_PUBKEY} && args.planData.pullers[2] == ${DEFAULT_PUBKEY} && args.planData.pullers[3] == ${DEFAULT_PUBKEY}`,
      },
      bind: {
        accounts: { plan: 'pda("subscriptions", "plan", wallet, args.planId)' },
        roles: { merchant: "wallet", puller: "args.planData.pullers[0]" },
      },
      build: [
        ix(subscriptions, "createPlan", {
          args: { planData: "args.planData" },
          accounts: {
            merchant: "wallet",
            planPda: 'pda("subscriptions", "plan", wallet, args.planId)',
            tokenMint: "args.mint",
          },
        }),
      ],
    },
    updatePlan: {
      scope: "instance",
      title: "Update the plan's status, end time or metadata",
      authority: { kind: "role", role: "merchant" },
      access: "secret",
      args: {
        updatePlanData: struct({
          status: { type: "integer", minimum: 0, maximum: 1, description: "0 is sunset, 1 is active." },
          endTs: { type: "string", format: "i64" },
          pullers: pubkeys,
          metadataUri,
          expectedCreatedAt: { type: "string", format: "i64" },
          expectedEndTs: { type: "string", format: "i64" },
          expectedPullers: pubkeys,
          expectedMetadataUri: metadataUri,
        }),
      },
      guards: {
        not_merchant: "wallet == instance.roles.merchant",
        plan_sunset: "reads.plan.status == 1",
        sunset_requires_end_time: "args.updatePlanData.status == 1 || args.updatePlanData.endTs != 0",
        pullers_changed:
          "args.updatePlanData.pullers[0] == reads.plan.data.pullers[0] && args.updatePlanData.pullers[1] == reads.plan.data.pullers[1] && args.updatePlanData.pullers[2] == reads.plan.data.pullers[2] && args.updatePlanData.pullers[3] == reads.plan.data.pullers[3]",
        stale_plan:
          "args.updatePlanData.expectedCreatedAt == reads.plan.data.terms.createdAt && args.updatePlanData.expectedEndTs == reads.plan.data.endTs",
      },
      reads: ["plan"],
      build: [
        ix(subscriptions, "updatePlan", {
          args: { updatePlanData: "args.updatePlanData" },
          accounts: { owner: "wallet", planPda: "instance.accounts.plan" },
        }),
      ],
    },
    collect: {
      scope: "instance",
      title: "Collect one period's amount from a subscriber",
      authority: { kind: "role", role: "merchant" },
      access: "secret",
      args: collectArgs,
      guards: { not_merchant: "wallet == instance.roles.merchant", ...collectGuards },
      reads: ["plan"],
      glue: { atas: [{ owner: "args.destination", mint: "params.mint" }] },
      build: collectBuild,
    },
    collectAsPuller: {
      scope: "instance",
      title: "Collect one period's amount as the plan's puller",
      authority: { kind: "role", role: "puller" },
      access: "secret",
      args: collectArgs,
      guards: {
        not_puller: "wallet == instance.roles.puller",
        puller_removed: "wallet in reads.plan.data.pullers",
        ...collectGuards,
      },
      reads: ["plan"],
      glue: { atas: [{ owner: "args.destination", mint: "params.mint" }] },
      build: collectBuild,
    },
    initAuthority: {
      scope: "participant",
      title: "Let the program pull this wallet's tokens",
      authority: "participant",
      access: "public",
      guards: { authority_exists: "!exists(reads.subscriptionAuthority.initId)" },
      reads: ["subscriptionAuthority"],
      glue: { atas: [{ owner: "wallet", mint: "params.mint" }] },
      build: [
        ix(subscriptions, "initSubscriptionAuthority", {
          accounts: {
            owner: "wallet",
            tokenMint: "params.mint",
            userAta: "ata(wallet, params.mint)",
            tokenProgram: TOKEN_PROGRAM,
          },
        }),
      ],
    },
    subscribe: {
      scope: "participant",
      title: "Subscribe to the plan",
      authority: "participant",
      access: "public",
      args: {
        subscribeData: struct({
          planId: { type: "string", format: "u64" },
          planBump: { type: "integer", minimum: 0, maximum: 255 },
          expectedMint: { type: "string", format: "pubkey" },
          expectedAmount: { type: "string", format: "u64" },
          expectedPeriodHours: { type: "string", format: "u64" },
          expectedCreatedAt: { type: "string", format: "i64" },
          expectedSubscriptionAuthorityInitId: { type: "string", format: "i64" },
        }),
      },
      guards: {
        already_subscribed: 'participant.state == "prospect"',
        plan_not_active: "reads.plan.status == 1",
        no_authority: "exists(reads.subscriptionAuthority.initId)",
        terms_mismatch:
          "args.subscribeData.planId == params.planId && args.subscribeData.planBump == reads.plan.bump && args.subscribeData.expectedMint == reads.plan.data.mint && args.subscribeData.expectedAmount == reads.plan.data.terms.amount && args.subscribeData.expectedPeriodHours == reads.plan.data.terms.periodHours && args.subscribeData.expectedCreatedAt == reads.plan.data.terms.createdAt",
        authority_mismatch:
          "args.subscribeData.expectedSubscriptionAuthorityInitId == reads.subscriptionAuthority.initId",
      },
      reads: ["plan", "subscriptionAuthority"],
      transitions: [{ machine: "participant", from: ["prospect"], to: "subscribed" }],
      build: [
        ix(subscriptions, "subscribe", {
          args: { subscribeData: "args.subscribeData" },
          accounts: {
            subscriber: "wallet",
            merchant: "instance.roles.merchant",
            planPda: "instance.accounts.plan",
            subscriptionAuthorityPda: 'pda("subscriptions", "subscriptionAuthority", wallet, params.mint)',
          },
        }),
      ],
    },
    cancel: {
      scope: "participant",
      title: "Cancel at the end of the current period",
      authority: "participant",
      access: "public",
      guards: { not_subscribed: 'participant.state == "subscribed"' },
      transitions: [{ machine: "participant", from: ["subscribed"], to: "canceled" }],
      build: [
        ix(subscriptions, "cancelSubscription", {
          accounts: { subscriber: "wallet", planPda: "instance.accounts.plan" },
        }),
      ],
    },
    resume: {
      scope: "participant",
      title: "Resume a canceled subscription before its period ends",
      authority: "participant",
      access: "public",
      args: { resumeData: struct({ expectedExpiresAtTs: { type: "string", format: "i64" } }) },
      guards: {
        not_canceled: 'participant.state == "canceled"',
        period_ended: "now < reads.subscription.expiresAtTs",
        stale_subscription: "args.resumeData.expectedExpiresAtTs == reads.subscription.expiresAtTs",
      },
      reads: ["subscription"],
      transitions: [{ machine: "participant", from: ["canceled"], to: "subscribed" }],
      build: [
        ix(subscriptions, "resumeSubscription", {
          args: { resumeData: "args.resumeData" },
          accounts: {
            subscriber: "wallet",
            planPda: "instance.accounts.plan",
            subscriptionAuthority: 'pda("subscriptions", "subscriptionAuthority", wallet, params.mint)',
          },
        }),
      ],
    },
  },
  // `collect` is left out: its acting wallet is `caller`, while these rows
  // are keyed by `subscriber`.
  stack: {
    instanceAccounts: { plan: { program: "subscriptions", account: "plan" } },
    activity: [
      { step: "subscribe", program: "subscriptions", instruction: "subscribe", instanceAccount: "planPda", walletAccount: "subscriber" },
      { step: "cancel", program: "subscriptions", instruction: "cancelSubscription", instanceAccount: "planPda", walletAccount: "subscriber" },
      { step: "resume", program: "subscriptions", instruction: "resumeSubscription", instanceAccount: "planPda", walletAccount: "subscriber" },
    ],
  },
});
