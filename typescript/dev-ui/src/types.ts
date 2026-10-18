/**
 * The shapes the dev UI reads. The spec types are a structural subset of
 * `@usearete/protocol`'s `ProtocolSpecV1` (only what the UI draws), so the
 * UI does not bundle the protocol package.
 */

export type MachineKind = "instance" | "participant";
export type StepScope = "create" | "instance" | "participant";

export type Authority =
  | { kind: "creator" }
  | { kind: "participant" }
  | { kind: "role"; role: string }
  | { kind: "keeper" }
  | { kind: "squadsProposer"; multisig: string };

export interface FieldSchema {
  type: "string" | "boolean" | "integer" | "array" | "object";
  format?: string;
  enum?: string[];
  maxLength?: number;
  minimum?: number;
  maximum?: number;
  items?: FieldSchema;
  maxItems?: number;
  properties?: Record<string, FieldSchema>;
  required?: string[];
  description?: string;
}

export interface ObjectSchema {
  type: "object";
  properties: Record<string, FieldSchema>;
  required: string[];
}

export interface Guard {
  code: string;
  expr: string;
  message?: string;
}

export interface Transition {
  machine: MachineKind;
  from: string[];
  to: string;
}

export type Build =
  | {
      kind: "template";
      instructions: Array<{
        program: string;
        instruction: string;
        args: Record<string, string>;
        accounts: Record<string, string>;
        remainingAccounts?: Array<{ pubkey: string; isWritable: boolean }>;
      }>;
    }
  | { kind: "handler"; export: string; programs: string[] };

export interface Step {
  scope: StepScope;
  title?: string;
  description?: string;
  authority: Authority;
  access?: "public" | "secret";
  args: ObjectSchema;
  guards: Guard[];
  reads: string[];
  ephemeral: string[];
  fee: { kind: "none" } | { kind: "input"; amountArg: string; asset: unknown } | { kind: "fixed" };
  transitions: Transition[];
  bind?: { accounts?: Record<string, string>; roles?: Record<string, string> };
  glue: { atas: Array<{ owner: string; mint: string }>; wrapSol?: { amount: string }; unwrapSol: boolean };
  build: Build;
}

export interface MachineState {
  kind: "recorded" | "derived";
  terminal: boolean;
  description?: string;
}

export interface Machine {
  initial: string;
  states: Record<string, MachineState>;
  derive: Array<{ from: string[]; to: string; when: string; at?: string }>;
}

export type Trigger =
  | { kind: "at"; machine: MachineKind; in: string[]; time: string }
  | { kind: "when"; machine: MachineKind; in: string[]; condition: string }
  | { kind: "onTransition"; machine: MachineKind; from: string[]; to: string[] }
  | { kind: "onConfirmed"; step: string };

export type Action =
  | { kind: "runKeeperStep"; step: string }
  | { kind: "proposeSquads"; step: string }
  | { kind: "emit"; event: string };

export interface Reaction {
  id: string;
  trigger: Trigger;
  action: Action;
}

export interface ProtocolSpec {
  name: string;
  version: string;
  description?: string;
  programs: Record<string, { kind: string; programId?: string }>;
  roles: Record<string, { description?: string }>;
  machines: { instance: Machine; participant?: Machine };
  steps: Record<string, Step>;
  reactions: Reaction[];
  events: Record<string, { machine: MachineKind; description?: string } | string>;
}

export interface MachineMapEdge {
  from: string;
  to: string;
  via: { step: string } | { derive: number };
}

export interface MachineMap {
  machines: Partial<Record<MachineKind, { initial: string; edges: MachineMapEdge[] }>>;
}

export type AnatomySlot =
  | { role: "runtime"; kind: "computeBudget" }
  | { role: "glue"; kind: "createAta"; owner: string; mint: string }
  | { role: "glue"; kind: "wrapSol"; amount: string }
  | { role: "runtime"; kind: "fee"; basis: "input" | "fixed"; placement: string }
  | { role: "program"; kind: "template"; program: string; instruction: string }
  | { role: "program"; kind: "handler"; export: string; programs: string[] }
  | { role: "glue"; kind: "unwrapSol" }
  | { role: "runtime"; kind: "memo" };

export interface Anatomy {
  steps: Record<string, { scope: StepScope; authority: Authority; instructions: AnatomySlot[] }>;
}

export interface Diagnostic {
  severity: "error" | "warning";
  message: string;
  code?: string | null;
  file?: string;
  line?: number;
  column?: number;
  path?: string;
}

export interface SectionDiff<T> {
  added: T[];
  removed: T[];
  changed: Array<T & { fields?: string[] }>;
}

export interface SpecDiff {
  initial: boolean;
  unchanged: boolean;
  steps: SectionDiff<{ name: string }>;
  states: SectionDiff<{ machine: MachineKind; name: string }>;
  /** `from` is null for an edge from no state (a create step's start). */
  transitions: SectionDiff<{ machine: MachineKind; from: string | null; to: string; via: string }>;
  reactions: SectionDiff<{ id: string }>;
  events: SectionDiff<{ name: string }>;
}

export interface BuildStatus {
  state: "pending" | "building" | "ok" | "failed";
  at?: string;
  error?: string;
  diagnostics?: Diagnostic[];
  diff?: SpecDiff;
  changed?: string[];
}

export interface Deployment {
  deploymentId?: string;
  releaseHash?: string;
  shortId?: string;
  slug?: string;
  stack?: string | null;
}

export interface Wallet {
  name: string;
  pubkey: string;
}

/** `GET /dev/v1/protocol`. */
export interface ProtocolSnapshot {
  boot: string;
  cursor: string;
  project?: string | null;
  spec: ProtocolSpec | null;
  machineMap: MachineMap | null;
  anatomy: Anatomy | null;
  build: BuildStatus;
  deployment: Deployment | null;
  /** The cloud dev environment the session runs against. */
  environment?: { id: string; name: string } | null;
  wallets: Wallet[];
  urls: Record<string, string>;
  chain: { slot?: number; unixTimestamp?: number } | null;
  stackError?: string | null;
  activitySource?: string;
}

/** An instance as the instances route lists it. */
export interface InstanceSummary {
  id: string;
  deploymentId: string;
  state: string;
  terminal?: boolean;
  createdAt?: string;
  updatedAt?: string;
  roles?: Record<string, string>;
  accounts?: Record<string, string>;
}

/** One event of the feed. */
export interface DevEvent {
  seq: number;
  type: string;
  at: string;
  data: Record<string, any>;
}
