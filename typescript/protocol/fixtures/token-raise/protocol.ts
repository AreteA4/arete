/**
 * The reference token raise, authored with `defineProtocol`. Lowering it
 * reproduces `test-vectors/protocol/token-raise.protocol-spec.json` exactly.
 */
import { defineProtocol } from "@usearete/protocol";

import { createLaunch } from "./handlers/createLaunch.js";

const presale = "instance.accounts.presale";
const vault = 'pda("squadsV4", "vault", resources.treasury.multisig, 0)';

export default defineProtocol({
  name: "tokenRaise",
  version: "1.0.0",
  description:
    "A token raise on Meteora Presale with a soft cap, team finalisation through Squads, and refunds.",
  programs: {
    meteoraPresale: {
      programId: "presSVxnf9UU8jMxhgSMqaRwNiT36qeBdNeTRKjTdbj",
      programSpecHash: "arete:h1:program-spec:sha256:a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1",
    },
    squadsV4: {
      programId: "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf",
      programSpecHash: "arete:h1:program-spec:sha256:b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2",
    },
    system: { builtin: "system" },
    splToken: { builtin: "splToken" },
  },
  stacks: {
    meteoraPresale: {
      stackManifestHash:
        "arete:h1:stack-manifest:sha256:c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3",
      package: "meteora-presale",
    },
  },
  params: {
    deadline: "unixTimestamp",
    softCap: "u64",
    hardCap: "u64",
    walletCap: "u64",
    quoteMint: "pubkey",
  },
  resources: {
    launchDefaults: {
      schema: {
        type: "object",
        properties: { tokenDecimals: { type: "integer", minimum: 0, maximum: 9 } },
        required: ["tokenDecimals"],
        additionalProperties: false,
      },
      description: "Publisher defaults for new launches.",
    },
    treasury: {
      schema: {
        type: "object",
        properties: { multisig: { type: "string", format: "pubkey" } },
        required: ["multisig"],
        additionalProperties: false,
      },
      description: "The team's Squads v4 multisig. Raises are finalised from its vault 0.",
    },
  },
  instanceVisibility: "account",
  reads: {
    presale: { kind: "account", program: "meteoraPresale", account: "Presale", address: presale },
    escrow: {
      kind: "account",
      program: "meteoraPresale",
      account: "Escrow",
      address: 'pda("meteoraPresale", "escrow", instance.accounts.presale, wallet)',
      optional: true,
    },
  },
  machines: {
    instance: {
      initial: "open",
      states: {
        open: { description: "Accepting contributions." },
        closed: "derived",
        succeeded: "derived",
        failed: { kind: "derived", terminal: true },
        awaiting_approval: "recorded",
        finalized: { kind: "derived", terminal: true },
      },
      derive: [
        {
          from: ["open"],
          to: "closed",
          when: "now >= params.deadline || reads.presale.totalDeposit >= params.hardCap",
          at: "params.deadline",
        },
        { from: ["closed"], to: "succeeded", when: "reads.presale.totalDeposit >= params.softCap" },
        { from: ["closed"], to: "failed", when: "reads.presale.totalDeposit < params.softCap" },
        { from: ["awaiting_approval"], to: "finalized", when: "reads.presale.creatorWithdrawn" },
      ],
      reads: ["presale"],
    },
    participant: {
      initial: "contributed",
      states: {
        contributed: "recorded",
        claimed: { terminal: true },
        refunded: { terminal: true },
      },
    },
  },
  steps: {
    create: {
      scope: "create",
      title: "Create a launch",
      authority: "creator",
      args: {
        deadline: "unixTimestamp",
        softCap: "u64",
        hardCap: "u64",
        walletCap: "u64",
        quoteMint: "pubkey",
      },
      guards: {
        soft_cap_above_hard_cap: {
          expr: "args.softCap <= args.hardCap",
          message: "The soft cap must not exceed the hard cap.",
        },
      },
      ephemeral: ["mint", "presaleBase"],
      handler: createLaunch,
      export: "createLaunch",
      programs: ["meteoraPresale", "splToken", "system"],
      bind: {
        accounts: {
          mint: "ephemeral.mint",
          presale: 'pda("meteoraPresale", "presale", ephemeral.presaleBase, ephemeral.mint, params.quoteMint)',
        },
      },
    },
    contribute: {
      scope: "participant",
      title: "Contribute",
      authority: "participant",
      args: { amount: "u64" },
      guards: {
        raise_not_open: 'instance.state == "open"',
        deadline_passed: "now < params.deadline",
        wallet_cap_exceeded: "(reads.escrow.totalDeposit ?? 0) + args.amount <= params.walletCap",
        hard_cap_exceeded: "reads.presale.totalDeposit + args.amount <= params.hardCap",
      },
      reads: ["escrow", "presale"],
      fee: { kind: "input", amountArg: "amount", asset: { kind: "native" } },
      glue: {
        atas: [{ owner: "wallet", mint: "params.quoteMint" }],
        wrapSol: { amount: "args.amount" },
      },
      build: [
        {
          program: "meteoraPresale",
          ix: "deposit",
          args: { max_amount: "args.amount" },
          accounts: { presale, owner: "wallet" },
        },
      ],
    },
    finalize: {
      scope: "instance",
      title: "Finalise through Squads",
      authority: { kind: "squadsProposer", multisig: "resources.treasury.multisig" },
      guards: { raise_not_succeeded: 'instance.state == "succeeded"' },
      transitions: [{ from: ["succeeded"], to: "awaiting_approval" }],
      build: [
        {
          program: "meteoraPresale",
          ix: "creator_withdraw",
          accounts: { presale, owner: vault },
        },
      ],
    },
    claim: {
      scope: "participant",
      authority: "participant",
      guards: { raise_not_finalized: 'instance.state == "finalized"' },
      transitions: [{ machine: "participant", from: ["contributed"], to: "claimed" }],
      glue: { atas: [{ owner: "wallet", mint: "instance.accounts.mint" }] },
      build: [{ program: "meteoraPresale", ix: "claim", accounts: { presale, owner: "wallet" } }],
    },
    refund: {
      scope: "participant",
      authority: "participant",
      guards: { raise_not_failed: 'instance.state == "failed"' },
      transitions: [{ machine: "participant", from: ["contributed"], to: "refunded" }],
      glue: { unwrapSol: true },
      build: [{ program: "meteoraPresale", ix: "withdraw", accounts: { presale, owner: "wallet" } }],
    },
  },
  reactions: [
    {
      id: "proposeFinalize",
      trigger: { kind: "onTransition", machine: "instance", from: [], to: ["succeeded"] },
      action: { kind: "proposeSquads", step: "finalize" },
    },
    {
      id: "announceSucceeded",
      trigger: { kind: "onTransition", machine: "instance", from: [], to: ["succeeded"] },
      action: { kind: "emit", event: "raise.succeeded" },
    },
    {
      id: "announceFailed",
      trigger: { kind: "onTransition", machine: "instance", from: [], to: ["failed"] },
      action: { kind: "emit", event: "raise.failed" },
    },
    {
      id: "announceContribution",
      trigger: { kind: "onConfirmed", step: "contribute" },
      action: { kind: "emit", event: "contribution.confirmed" },
    },
  ],
  events: {
    "raise.succeeded": "instance",
    "raise.failed": "instance",
    "contribution.confirmed": { machine: "participant", description: "A contribution confirmed on chain." },
  },
  metadata: {
    instance: {
      fields: {
        name: { schema: { type: "string", maxLength: 64 }, writer: "creator", filterable: true },
        ticker: { schema: { type: "string", maxLength: 10 }, writer: "creator", filterable: true },
        tagline: { schema: { type: "string", maxLength: 280 }, writer: "creator", filterable: false },
      },
      maxBytes: 4096,
    },
  },
  stack: {
    instanceAccounts: { presale: { program: "meteoraPresale", account: "Presale" } },
    instanceFields: { totalDeposit: { account: "presale", field: "total_deposit" } },
    activity: [
      {
        step: "contribute",
        program: "meteoraPresale",
        instruction: "deposit",
        instanceAccount: "presale",
        walletAccount: "owner",
        amountArg: "max_amount",
      },
    ],
  },
});
