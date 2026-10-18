import type { ProtocolSnapshot, ProtocolSpec, Step } from "./types";

const transfer = (exportName: string): Step["build"] => ({ kind: "handler", export: exportName, programs: ["system"] });

const base = {
  reads: [],
  ephemeral: [],
  fee: { kind: "none" as const },
  glue: { atas: [], unwrapSol: false },
};

/** A tip jar with a derived `expired` state and a participant machine. */
export const tipJar: ProtocolSpec = {
  name: "tipJar",
  version: "0.1.0",
  programs: { system: { kind: "programSpec" } },
  roles: { recipient: {} },
  machines: {
    instance: {
      initial: "open",
      states: {
        open: { kind: "recorded", terminal: false },
        closed: { kind: "recorded", terminal: true },
        expired: { kind: "derived", terminal: true },
      },
      derive: [{ from: ["open"], to: "expired", when: "clock.unixTimestamp > params.deadline" }],
    },
    participant: {
      initial: "joined",
      states: { joined: { kind: "recorded", terminal: false }, left: { kind: "recorded", terminal: true } },
      derive: [],
    },
  },
  steps: {
    open: {
      ...base,
      scope: "create",
      title: "Open a tip jar",
      authority: { kind: "creator" },
      access: "secret",
      args: {
        type: "object",
        properties: { recipient: { type: "string", format: "pubkey" }, amount: { type: "string", format: "u64" } },
        required: ["recipient", "amount"],
      },
      guards: [{ code: "amount_zero", expr: "args.amount > 0" }],
      transitions: [],
      build: transfer("openJar"),
    },
    close: {
      ...base,
      scope: "instance",
      authority: { kind: "role", role: "recipient" },
      args: { type: "object", properties: { amount: { type: "string", format: "u64" } }, required: ["amount"] },
      guards: [
        { code: "not_recipient", expr: "wallet == instance.roles.recipient" },
        { code: "not_open", expr: 'instance.state == "open"' },
      ],
      transitions: [{ machine: "instance", from: ["open"], to: "closed" }],
      build: {
        kind: "template",
        instructions: [
          {
            program: "system",
            instruction: "transfer",
            args: { lamports: "args.amount" },
            accounts: { from: "wallet", to: "instance.accounts.creator" },
          },
        ],
      },
    },
    join: {
      ...base,
      scope: "participant",
      authority: { kind: "participant" },
      args: { type: "object", properties: {}, required: [] },
      guards: [],
      transitions: [{ machine: "participant", from: [], to: "joined" }],
      build: transfer("joinJar"),
    },
    poke: {
      ...base,
      scope: "instance",
      authority: { kind: "keeper" },
      args: { type: "object", properties: {}, required: [] },
      guards: [],
      transitions: [],
      build: transfer("pokeJar"),
    },
  },
  reactions: [
    {
      id: "announceClosed",
      trigger: { kind: "onTransition", machine: "instance", from: [], to: ["closed"] },
      action: { kind: "emit", event: "tipJar.closed" },
    },
  ],
  events: { "tipJar.closed": { machine: "instance" } },
};

export function snapshot(overrides: Partial<ProtocolSnapshot> = {}): ProtocolSnapshot {
  return {
    boot: "b00t",
    cursor: "b00t:0",
    spec: tipJar,
    machineMap: null,
    anatomy: null,
    build: { state: "ok" },
    deployment: { deploymentId: "d1" },
    wallets: [{ name: "alice", pubkey: "A1ice" }],
    urls: {},
    chain: null,
    ...overrides,
  };
}
