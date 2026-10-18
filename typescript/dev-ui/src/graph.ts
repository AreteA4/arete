/**
 * Spec → diagram graph. Pure: no layout, no React, no local-dev
 * assumptions, so a hosted read-only view can reuse it.
 *
 * - Each machine (instance, participant) is a lane of state nodes; derived
 *   states are marked so the UI can dash them.
 * - Steps are edges labelled with their authority and guards. Create steps
 *   (and transitions with no source state) start from the lane's start node.
 *   Steps that move no state are listed as loose step nodes.
 * - Derivation rules are dashed edges.
 * - Reactions and the events they emit are side nodes, wired to the states
 *   that trigger them.
 */

import type {
  Authority,
  Guard,
  MachineKind,
  MachineMap,
  ProtocolSpec,
  Reaction,
  SpecDiff,
  Step,
} from "./types";

export type NodeKind = "state" | "start" | "reaction" | "event" | "step";
export type EdgeKind = "step" | "derive" | "trigger" | "emit";

export interface GraphNode {
  id: string;
  kind: NodeKind;
  lane: MachineKind | "side";
  label: string;
  data: {
    machine?: MachineKind;
    state?: string;
    derived?: boolean;
    terminal?: boolean;
    initial?: boolean;
    reaction?: Reaction;
    event?: string;
    step?: string;
    description?: string;
  };
}

export interface GraphEdge {
  id: string;
  kind: EdgeKind;
  source: string;
  target: string;
  label: string;
  dashed: boolean;
  data: {
    machine?: MachineKind;
    from?: string | null;
    to?: string;
    step?: string;
    derive?: number;
    authority?: string;
    guards?: Guard[];
    when?: string;
  };
}

export interface Graph {
  nodes: GraphNode[];
  edges: GraphEdge[];
}

export const stateId = (machine: MachineKind, state: string) => `state:${machine}:${state}`;
export const startId = (machine: MachineKind) => `start:${machine}`;
export const reactionId = (id: string) => `reaction:${id}`;
export const eventId = (name: string) => `event:${name}`;
export const looseStepId = (step: string) => `step:${step}`;

export function stepEdgeId(machine: MachineKind, from: string | null, to: string, step: string): string {
  return `edge:${machine}:${from ?? "*"}>${to}:step:${step}`;
}

export function deriveEdgeId(machine: MachineKind, from: string, to: string, index: number): string {
  return `edge:${machine}:${from}>${to}:derive:${index}`;
}

export function authorityLabel(authority: Authority): string {
  switch (authority.kind) {
    case "creator":
      return "creator";
    case "participant":
      return "participant";
    case "role":
      return `role ${authority.role}`;
    case "keeper":
      return "keeper";
    case "squadsProposer":
      return "Squads proposer";
  }
}

/** The signer classes a step's transaction carries. */
export function signerClasses(step: Step): string[] {
  const authority = step.authority.kind;
  const main =
    authority === "keeper" ? "keeper" : authority === "squadsProposer" ? "squadsProposer" : "wallet";
  const classes = [main];
  if (step.ephemeral.length > 0) classes.push("ephemeral");
  classes.push("attestation");
  return classes;
}

function stepLabel(name: string, step: Step): string {
  const guards = step.guards.length > 0 ? ` · ${step.guards.length} guard${step.guards.length === 1 ? "" : "s"}` : "";
  return `${name} (${authorityLabel(step.authority)}${guards})`;
}

function machinesOf(spec: ProtocolSpec): MachineKind[] {
  return spec.machines.participant ? ["instance", "participant"] : ["instance"];
}

/** Every step's state moves, from the machine map when there is one. */
function stepMoves(spec: ProtocolSpec, map: MachineMap | null) {
  const moves: Array<{ machine: MachineKind; from: string | null; to: string; step: string }> = [];
  const derives: Array<{ machine: MachineKind; from: string; to: string; index: number }> = [];
  for (const machine of machinesOf(spec)) {
    const mapped = map?.machines?.[machine];
    if (mapped) {
      for (const edge of mapped.edges) {
        if ("step" in edge.via) moves.push({ machine, from: edge.from, to: edge.to, step: edge.via.step });
        else derives.push({ machine, from: edge.from, to: edge.to, index: edge.via.derive });
      }
    } else {
      for (const [name, step] of Object.entries(spec.steps)) {
        for (const transition of step.transitions ?? []) {
          if (transition.machine !== machine) continue;
          for (const from of transition.from) {
            if (from !== transition.to) moves.push({ machine, from, to: transition.to, step: name });
          }
        }
      }
      spec.machines[machine]?.derive?.forEach((rule, index) => {
        for (const from of rule.from) {
          if (from !== rule.to) derives.push({ machine, from, to: rule.to, index });
        }
      });
    }
    // Transitions without a source state start the machine.
    for (const [name, step] of Object.entries(spec.steps)) {
      for (const transition of step.transitions ?? []) {
        if (transition.machine === machine && transition.from.length === 0) {
          moves.push({ machine, from: null, to: transition.to, step: name });
        }
      }
    }
  }
  // Create steps start the instance machine at its initial state.
  for (const [name, step] of Object.entries(spec.steps)) {
    if (step.scope === "create" && !moves.some((move) => move.step === name && move.from === null)) {
      moves.push({ machine: "instance", from: null, to: spec.machines.instance.initial, step: name });
    }
  }
  return { moves, derives };
}

function reactionLabel(reaction: Reaction): string {
  const action =
    reaction.action.kind === "emit"
      ? `emit ${reaction.action.event}`
      : reaction.action.kind === "runKeeperStep"
        ? `run ${reaction.action.step}`
        : `propose ${reaction.action.step}`;
  return `${reaction.id}: ${action}`;
}

/** The states whose entry (or presence) triggers a reaction. */
function triggerStates(spec: ProtocolSpec, reaction: Reaction): Array<{ machine: MachineKind; state: string }> {
  const trigger = reaction.trigger;
  switch (trigger.kind) {
    case "onTransition": {
      const machine = spec.machines[trigger.machine];
      const states = trigger.to.length > 0 ? trigger.to : Object.keys(machine?.states ?? {});
      return states.map((state) => ({ machine: trigger.machine, state }));
    }
    case "at":
    case "when":
      return trigger.in.map((state) => ({ machine: trigger.machine, state }));
    case "onConfirmed":
      return [];
  }
}

export function buildGraph(spec: ProtocolSpec, map: MachineMap | null = null): Graph {
  const nodes: GraphNode[] = [];
  const edges: GraphEdge[] = [];
  const { moves, derives } = stepMoves(spec, map);

  for (const machine of machinesOf(spec)) {
    const definition = spec.machines[machine]!;
    if (moves.some((move) => move.machine === machine && move.from === null)) {
      nodes.push({ id: startId(machine), kind: "start", lane: machine, label: "start", data: { machine } });
    }
    for (const [name, state] of Object.entries(definition.states)) {
      nodes.push({
        id: stateId(machine, name),
        kind: "state",
        lane: machine,
        label: name,
        data: {
          machine,
          state: name,
          derived: state.kind === "derived",
          terminal: state.terminal,
          initial: definition.initial === name,
          description: state.description,
        },
      });
    }
  }

  const seen = new Set<string>();
  for (const move of moves) {
    const step = spec.steps[move.step];
    if (!step) continue;
    const id = stepEdgeId(move.machine, move.from, move.to, move.step);
    if (seen.has(id)) continue;
    seen.add(id);
    edges.push({
      id,
      kind: "step",
      source: move.from === null ? startId(move.machine) : stateId(move.machine, move.from),
      target: stateId(move.machine, move.to),
      label: stepLabel(move.step, step),
      dashed: false,
      data: {
        machine: move.machine,
        from: move.from,
        to: move.to,
        step: move.step,
        authority: authorityLabel(step.authority),
        guards: step.guards,
      },
    });
  }
  for (const derive of derives) {
    const rule = spec.machines[derive.machine]?.derive?.[derive.index];
    edges.push({
      id: deriveEdgeId(derive.machine, derive.from, derive.to, derive.index),
      kind: "derive",
      source: stateId(derive.machine, derive.from),
      target: stateId(derive.machine, derive.to),
      label: rule?.when ? `derive: ${rule.when}` : "derive",
      dashed: true,
      data: { machine: derive.machine, from: derive.from, to: derive.to, derive: derive.index, when: rule?.when },
    });
  }

  // Steps that move no state.
  for (const [name, step] of Object.entries(spec.steps)) {
    if (!moves.some((move) => move.step === name)) {
      nodes.push({
        id: looseStepId(name),
        kind: "step",
        lane: step.scope === "participant" && spec.machines.participant ? "participant" : "instance",
        label: stepLabel(name, step),
        data: { step: name },
      });
    }
  }

  const events = new Set(Object.keys(spec.events ?? {}));
  for (const reaction of spec.reactions ?? []) {
    nodes.push({
      id: reactionId(reaction.id),
      kind: "reaction",
      lane: "side",
      label: reactionLabel(reaction),
      data: { reaction },
    });
    for (const { machine, state } of triggerStates(spec, reaction)) {
      if (!spec.machines[machine]?.states[state]) continue;
      edges.push({
        id: `trigger:${reaction.id}:${machine}:${state}`,
        kind: "trigger",
        source: stateId(machine, state),
        target: reactionId(reaction.id),
        label: reaction.trigger.kind,
        dashed: true,
        data: { machine, to: state },
      });
    }
    if (reaction.trigger.kind === "onConfirmed") {
      const step = reaction.trigger.step;
      const loose = nodes.find((node) => node.id === looseStepId(step));
      const target = loose
        ? loose.id
        : edges.find((edge) => edge.kind === "step" && edge.data.step === step)?.target;
      if (target) {
        edges.push({
          id: `trigger:${reaction.id}:confirmed:${step}`,
          kind: "trigger",
          source: target,
          target: reactionId(reaction.id),
          label: `${step} confirmed`,
          dashed: true,
          data: { step },
        });
      }
    }
    if (reaction.action.kind === "emit") events.add(reaction.action.event);
  }
  for (const name of events) {
    nodes.push({ id: eventId(name), kind: "event", lane: "side", label: name, data: { event: name } });
  }
  for (const reaction of spec.reactions ?? []) {
    if (reaction.action.kind === "emit") {
      edges.push({
        id: `emit:${reaction.id}:${reaction.action.event}`,
        kind: "emit",
        source: reactionId(reaction.id),
        target: eventId(reaction.action.event),
        label: "emits",
        dashed: true,
        data: {},
      });
    }
  }
  return { nodes, edges };
}

/** The step edges a transition took: the step's own edge, else any step or derive edge between the states. */
export function edgesForTransition(
  graph: Graph,
  machine: MachineKind,
  from: string | null,
  to: string,
  cause: { kind?: string; ref?: string } | null,
): string[] {
  const between = graph.edges.filter(
    (edge) =>
      (edge.kind === "step" || edge.kind === "derive") &&
      edge.data.machine === machine &&
      (edge.data.from ?? null) === from &&
      edge.data.to === to,
  );
  if (cause?.kind === "step" && cause.ref) {
    const own = between.filter((edge) => edge.data.step === cause.ref);
    if (own.length > 0) return own.map((edge) => edge.id);
  }
  if (cause?.kind === "derive" || cause?.kind === "timer") {
    const derived = between.filter((edge) => edge.kind === "derive");
    if (derived.length > 0) return derived.map((edge) => edge.id);
  }
  return between.map((edge) => edge.id);
}

/** Nodes and edges a rebuild added or changed, for highlighting. */
export function changedIds(graph: Graph, diff: SpecDiff | undefined | null): Set<string> {
  const ids = new Set<string>();
  if (!diff || diff.initial || diff.unchanged) return ids;
  for (const state of [...diff.states.added, ...diff.states.changed]) ids.add(stateId(state.machine, state.name));
  const steps = new Set([...diff.steps.added, ...diff.steps.changed].map((step) => step.name));
  for (const edge of graph.edges) {
    if (edge.data.step && steps.has(edge.data.step) && edge.kind === "step") ids.add(edge.id);
  }
  for (const node of graph.nodes) {
    if (node.kind === "step" && node.data.step && steps.has(node.data.step)) ids.add(node.id);
  }
  for (const transition of diff.transitions.added) {
    const [kind, ref] = transition.via.split(":");
    for (const edge of graph.edges) {
      if (
        edge.data.machine === transition.machine &&
        (edge.data.from ?? null) === (transition.from ?? null) &&
        edge.data.to === transition.to &&
        ((kind === "step" && edge.data.step === ref) || (kind === "derive" && String(edge.data.derive) === ref))
      ) {
        ids.add(edge.id);
      }
    }
  }
  for (const reaction of [...diff.reactions.added, ...diff.reactions.changed]) ids.add(reactionId(reaction.id));
  for (const event of diff.events.added) ids.add(eventId(event.name));
  return ids;
}

/** Instance counts per state node id. */
export function stateCounts(instances: Array<{ state: string }>, machine: MachineKind = "instance"): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const instance of instances) {
    const id = stateId(machine, instance.state);
    counts[id] = (counts[id] ?? 0) + 1;
  }
  return counts;
}

/** The summary line of a diff, e.g. `+1 step, ~1 step, +1 state`. */
export function diffSummary(diff: SpecDiff | undefined | null): string {
  if (!diff) return "";
  if (diff.initial) return "first build";
  if (diff.unchanged) return "no changes";
  const parts: string[] = [];
  const add = (count: number, sign: string, noun: string) => {
    if (count > 0) parts.push(`${sign}${count} ${noun}${count === 1 ? "" : "s"}`);
  };
  for (const [key, noun] of [
    ["steps", "step"],
    ["states", "state"],
    ["transitions", "transition"],
    ["reactions", "reaction"],
    ["events", "event"],
  ] as const) {
    const section = diff[key];
    add(section.added.length, "+", noun);
    add(section.removed.length, "−", noun);
    add(section.changed.length, "~", noun);
  }
  return parts.join(", ") || "no structural changes";
}
