/**
 * The UI's state, folded from the protocol snapshot, the instance list and
 * the event feed. Pure reducer; the components derive counts, highlights
 * and animations from it.
 */

import type { DevEvent, InstanceSummary, MachineKind, ProtocolSnapshot } from "./types";

export type Connection = "connecting" | "live" | "reconnecting" | "offline";

export interface Execution {
  key: string;
  executionId: string | null;
  step: string;
  state: string;
  instanceId: string | null;
  deploymentId: string | null;
  wallet: string | null;
  devWallet?: string | null;
  args: Record<string, unknown> | null;
  transactions: Array<{ index: number; state: string; signature?: string | null; error?: string | null }>;
  simulation: Array<{ transaction?: number; err?: unknown; logs?: string[] }>;
  error: unknown;
  rejected?: string | null;
  source: string;
  updates: Array<{ type: string; state: string; at: string }>;
  firstAt: string;
  lastAt: string;
}

export interface TimelineEntry {
  seq: number;
  at: string;
  kind: "transition" | "step" | "emit" | "stack";
  label: string;
  machine?: MachineKind;
  from?: string | null;
  to?: string;
  cause?: { kind?: string; ref?: string } | null;
  ok?: boolean;
}

export interface TrackedInstance extends InstanceSummary {
  history: TimelineEntry[];
}

/** Something the diagram animates briefly. */
export type Pulse =
  | { id: number; at: number; kind: "transition"; machine: MachineKind; from: string | null; to: string; cause: { kind?: string; ref?: string } | null }
  | { id: number; at: number; kind: "emit"; event: string; reaction?: string }
  | { id: number; at: number; kind: "step"; step: string; ok: boolean };

export interface Model {
  snapshot: ProtocolSnapshot | null;
  connection: Connection;
  /** The replay is done; new events are live (and animate). */
  caughtUp: boolean;
  instances: Record<string, TrackedInstance>;
  executions: Record<string, Execution>;
  /** Execution keys, newest first. */
  executionOrder: string[];
  /** The newest events first. */
  log: DevEvent[];
  pulses: Pulse[];
  chain: { slot?: number; unixTimestamp?: number } | null;
  /** The snapshot or instances are stale (a rebuild, a deploy, a reset). */
  needsReload: boolean;
  degraded: string | null;
  lastSeq: number;
}

export const LOG_LIMIT = 300;
export const EXECUTION_LIMIT = 200;
export const HISTORY_LIMIT = 100;
export const PULSE_LIMIT = 40;

export const initialModel: Model = {
  snapshot: null,
  connection: "connecting",
  caughtUp: false,
  instances: {},
  executions: {},
  executionOrder: [],
  log: [],
  pulses: [],
  chain: null,
  needsReload: false,
  degraded: null,
  lastSeq: 0,
};

export type ModelAction =
  | { type: "snapshot"; snapshot: ProtocolSnapshot }
  | { type: "instances"; instances: InstanceSummary[] }
  | { type: "event"; event: DevEvent; now?: number }
  | { type: "connection"; connection: Connection }
  | { type: "live" }
  | { type: "reloaded" };

function pushCapped<T>(list: T[], item: T, limit: number): T[] {
  const next = [item, ...list];
  return next.length > limit ? next.slice(0, limit) : next;
}

function appendHistory(history: TimelineEntry[], entry: TimelineEntry): TimelineEntry[] {
  if (history.some((existing) => existing.seq === entry.seq)) return history;
  const next = [...history, entry];
  return next.length > HISTORY_LIMIT ? next.slice(next.length - HISTORY_LIMIT) : next;
}

let pulseCounter = 0;

type PulseInput =
  | Omit<Extract<Pulse, { kind: "transition" }>, "id" | "at">
  | Omit<Extract<Pulse, { kind: "emit" }>, "id" | "at">
  | Omit<Extract<Pulse, { kind: "step" }>, "id" | "at">;

/** Live events animate; replays (not caught up yet) and history do not. */
function addPulse(model: Model, pulse: PulseInput, now: number, history = false): Pulse[] {
  if (!model.caughtUp || history) return model.pulses;
  pulseCounter += 1;
  return [{ ...pulse, id: pulseCounter, at: now } as Pulse, ...model.pulses].slice(0, PULSE_LIMIT);
}

function withInstanceEntry(model: Model, instanceId: string | null | undefined, entry: TimelineEntry, patch?: Partial<InstanceSummary>): Record<string, TrackedInstance> {
  if (!instanceId) return model.instances;
  const existing = model.instances[instanceId];
  const base: TrackedInstance = existing ?? {
    id: instanceId,
    deploymentId: (patch?.deploymentId as string) ?? "",
    state: patch?.state ?? "",
    history: [],
  };
  return {
    ...model.instances,
    [instanceId]: { ...base, ...patch, history: appendHistory(base.history, entry) },
  };
}

function stepEvent(model: Model, event: DevEvent, now: number): Model {
  const data = event.data;
  const key: string = data.executionId ?? `rejected:${event.seq}`;
  const previous = model.executions[key];
  const state: string = data.state ?? event.type.replace("step.", "");
  const execution: Execution = {
    key,
    executionId: data.executionId ?? null,
    step: data.step ?? previous?.step ?? "?",
    state,
    instanceId: data.instanceId ?? previous?.instanceId ?? null,
    deploymentId: data.deploymentId ?? previous?.deploymentId ?? null,
    wallet: data.wallet ?? previous?.wallet ?? null,
    devWallet: data.devWallet ?? previous?.devWallet ?? null,
    args: data.args ?? previous?.args ?? null,
    transactions: data.transactions ?? previous?.transactions ?? [],
    simulation: data.simulation ?? previous?.simulation ?? [],
    error: data.error ?? previous?.error ?? null,
    rejected: data.rejected ?? previous?.rejected ?? null,
    source: data.source ?? previous?.source ?? "runtime",
    updates: [...(previous?.updates ?? []), { type: event.type, state, at: event.at }],
    firstAt: previous?.firstAt ?? event.at,
    lastAt: event.at,
  };
  const executions = { ...model.executions, [key]: execution };
  let executionOrder = previous ? model.executionOrder : [key, ...model.executionOrder];
  if (executionOrder.length > EXECUTION_LIMIT) {
    for (const dropped of executionOrder.slice(EXECUTION_LIMIT)) delete executions[dropped];
    executionOrder = executionOrder.slice(0, EXECUTION_LIMIT);
  }
  const terminal = event.type === "step.confirmed" || event.type === "step.failed";
  const ok = event.type === "step.confirmed";
  const instances = terminal
    ? withInstanceEntry(model, execution.instanceId, {
        seq: event.seq,
        at: event.at,
        kind: "step",
        label: `${execution.step} ${ok ? "confirmed" : execution.rejected ? `rejected (${execution.rejected})` : state}`,
        ok,
      })
    : model.instances;
  const pulses = terminal ? addPulse(model, { kind: "step", step: execution.step, ok }, now, data.history === true) : model.pulses;
  return { ...model, executions, executionOrder, instances, pulses };
}

export function reduce(model: Model, action: ModelAction): Model {
  switch (action.type) {
    case "snapshot":
      return { ...model, snapshot: action.snapshot, chain: action.snapshot.chain ?? model.chain };
    case "instances": {
      const instances = { ...model.instances };
      for (const instance of action.instances) {
        const existing = instances[instance.id];
        instances[instance.id] = { ...instance, history: existing?.history ?? [] };
      }
      return { ...model, instances };
    }
    case "connection":
      return {
        ...model,
        connection: action.connection,
        caughtUp: action.connection === "live" ? model.caughtUp : false,
      };
    case "live":
      return { ...model, connection: "live", caughtUp: true };
    case "reloaded":
      return { ...model, needsReload: false };
    case "event":
      return applyEvent(model, action.event, action.now ?? Date.now());
  }
}

function applyEvent(input: Model, event: DevEvent, now: number): Model {
  // Replays after a reconnect repeat events already folded in.
  if (event.seq > 0 && event.seq <= input.lastSeq && event.type !== "stream.reset") return input;
  let model: Model = {
    ...input,
    lastSeq: event.seq > 0 ? event.seq : input.lastSeq,
    log: event.type === "chain.clock" && event.seq === 0 ? input.log : pushCapped(input.log, event, LOG_LIMIT),
  };
  const data = event.data ?? {};
  switch (event.type) {
    case "stream.reset":
      return { ...model, lastSeq: 0, needsReload: true };
    case "build.started":
    case "build.failed":
      if (!model.snapshot) return model;
      return {
        ...model,
        snapshot: {
          ...model.snapshot,
          build:
            event.type === "build.started"
              ? { ...model.snapshot.build, state: "building", changed: data.changed }
              : { state: "failed", at: event.at, error: data.error, diagnostics: data.diagnostics },
        },
      };
    case "build.succeeded":
    case "deploy.activated":
    case "deploy.failed":
      return { ...model, needsReload: true };
    case "feed.degraded":
      return { ...model, degraded: data.reason ?? "degraded" };
    case "chain.clock":
      return { ...model, chain: { slot: data.slot, unixTimestamp: data.unixTimestamp } };
    case "step.prepared":
    case "step.submitted":
    case "step.confirmed":
    case "step.failed":
      return stepEvent(model, event, now);
    case "instance.transitioned": {
      const machine: MachineKind = data.machine === "participant" ? "participant" : "instance";
      const entry: TimelineEntry = {
        seq: event.seq,
        at: data.at ?? event.at,
        kind: "transition",
        machine,
        from: data.from ?? null,
        to: data.to,
        cause: data.cause ?? null,
        label: `${machine === "participant" ? `participant ${short(data.wallet)} ` : ""}${data.from ?? "∅"} → ${data.to}${data.cause?.ref ? ` (${data.cause.kind} ${data.cause.ref})` : ""}`,
      };
      const patch: Partial<InstanceSummary> | undefined =
        machine === "instance"
          ? { state: data.to, deploymentId: data.deploymentId, updatedAt: data.at ?? event.at }
          : undefined;
      if (patch && !model.instances[data.instanceId]) patch.createdAt = data.at ?? event.at;
      model = {
        ...model,
        instances: withInstanceEntry(model, data.instanceId, entry, patch),
        pulses: addPulse(model, { kind: "transition", machine, from: data.from ?? null, to: data.to, cause: data.cause ?? null }, now, data.history === true),
      };
      return model;
    }
    case "instance.emitted":
      return {
        ...model,
        instances: withInstanceEntry(model, data.instanceId, {
          seq: event.seq,
          at: data.at ?? event.at,
          kind: "emit",
          label: `emitted ${data.event} (${data.reaction})`,
        }),
        pulses: addPulse(model, { kind: "emit", event: data.event, reaction: data.reaction }, now, data.history === true),
      };
    case "stack.row":
      return model;
    default:
      return model;
  }
}

function short(value: unknown): string {
  return typeof value === "string" && value.length > 10 ? `${value.slice(0, 4)}…${value.slice(-4)}` : String(value ?? "");
}

/** Instances of the active deployment. */
export function activeInstances(model: Model): TrackedInstance[] {
  const deployment = model.snapshot?.deployment?.deploymentId;
  return Object.values(model.instances).filter((instance) => !deployment || instance.deploymentId === deployment);
}

/** The executions of one step, newest first. */
export function executionsOf(model: Model, step: string, limit = 10): Execution[] {
  return model.executionOrder
    .map((key) => model.executions[key])
    .filter((execution): execution is Execution => execution !== undefined && execution.step === step)
    .slice(0, limit);
}
