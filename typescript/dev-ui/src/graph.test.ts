import { describe, expect, it } from "vitest";

import { tipJar } from "./fixtures";
import {
  buildGraph,
  changedIds,
  deriveEdgeId,
  diffSummary,
  edgesForTransition,
  looseStepId,
  signerClasses,
  startId,
  stateCounts,
  stateId,
  stepEdgeId,
} from "./graph";
import type { SpecDiff } from "./types";

describe("buildGraph", () => {
  const graph = buildGraph(tipJar);
  const node = (id: string) => graph.nodes.find((candidate) => candidate.id === id);
  const edge = (id: string) => graph.edges.find((candidate) => candidate.id === id);

  it("draws each machine's states, with derived states marked", () => {
    expect(node(stateId("instance", "open"))?.data).toMatchObject({ initial: true, derived: false });
    expect(node(stateId("instance", "expired"))?.data).toMatchObject({ derived: true, terminal: true });
    expect(node(stateId("participant", "joined"))?.lane).toBe("participant");
  });

  it("draws steps as edges labelled with authority and guards", () => {
    const close = edge(stepEdgeId("instance", "open", "closed", "close"));
    expect(close).toBeDefined();
    expect(close?.label).toBe("close (role recipient · 2 guards)");
    expect(close?.data.guards?.map((guard) => guard.code)).toEqual(["not_recipient", "not_open"]);
    // Create steps start at the initial state.
    const open = edge(stepEdgeId("instance", null, "open", "open"));
    expect(open?.source).toBe(startId("instance"));
    // A participant transition without a source starts the participant lane.
    expect(edge(stepEdgeId("participant", null, "joined", "join"))?.source).toBe(startId("participant"));
  });

  it("draws derivations dashed and steps that move nothing as loose nodes", () => {
    const derive = edge(deriveEdgeId("instance", "open", "expired", 0));
    expect(derive?.dashed).toBe(true);
    expect(derive?.label).toContain("clock.unixTimestamp");
    expect(node(looseStepId("poke"))?.kind).toBe("step");
  });

  it("wires reactions to their trigger states and emitted events", () => {
    expect(node("reaction:announceClosed")?.label).toBe("announceClosed: emit tipJar.closed");
    expect(edge("trigger:announceClosed:instance:closed")?.source).toBe(stateId("instance", "closed"));
    expect(edge("emit:announceClosed:tipJar.closed")?.target).toBe("event:tipJar.closed");
  });

  it("prefers the machine map's edges", () => {
    const mapped = buildGraph(tipJar, {
      machines: { instance: { initial: "open", edges: [{ from: "open", to: "closed", via: { step: "close" } }] } },
    });
    expect(mapped.edges.some((candidate) => candidate.kind === "derive" && candidate.data.machine === "instance")).toBe(false);
  });
});

describe("highlights", () => {
  const graph = buildGraph(tipJar);

  it("finds the edge a transition took", () => {
    expect(edgesForTransition(graph, "instance", "open", "closed", { kind: "step", ref: "close" })).toEqual([
      stepEdgeId("instance", "open", "closed", "close"),
    ]);
    expect(edgesForTransition(graph, "instance", null, "open", { kind: "step", ref: "open" })).toEqual([
      stepEdgeId("instance", null, "open", "open"),
    ]);
    expect(edgesForTransition(graph, "instance", "open", "expired", { kind: "derive", ref: "0" })).toEqual([
      deriveEdgeId("instance", "open", "expired", 0),
    ]);
  });

  it("marks what a rebuild added or changed", () => {
    const empty = { added: [], removed: [], changed: [] };
    const diff: SpecDiff = {
      initial: false,
      unchanged: false,
      steps: { added: [], removed: [], changed: [{ name: "close", fields: ["guards"] }] },
      states: { added: [{ machine: "instance", name: "expired" }], removed: [], changed: [] },
      transitions: empty,
      reactions: empty,
      events: { added: [{ name: "tipJar.closed" }], removed: [], changed: [] },
    };
    const ids = changedIds(graph, diff);
    expect(ids).toContain(stateId("instance", "expired"));
    expect(ids).toContain(stepEdgeId("instance", "open", "closed", "close"));
    expect(ids).toContain("event:tipJar.closed");
    expect(changedIds(graph, { ...diff, initial: true }).size).toBe(0);
    expect(diffSummary(diff)).toBe("~1 step, +1 state, +1 event");
  });

  it("marks an added start edge, whose source the diff sends as null", () => {
    const empty = { added: [], removed: [], changed: [] };
    const diff: SpecDiff = {
      initial: false,
      unchanged: false,
      steps: empty,
      states: empty,
      transitions: { added: [{ machine: "instance", from: null, to: "open", via: "step:open" }], removed: [], changed: [] },
      reactions: empty,
      events: empty,
    };
    expect([...changedIds(graph, diff)]).toEqual([stepEdgeId("instance", null, "open", "open")]);
  });

  it("counts instances per state and lists signer classes", () => {
    expect(stateCounts([{ state: "open" }, { state: "open" }, { state: "closed" }])).toEqual({
      [stateId("instance", "open")]: 2,
      [stateId("instance", "closed")]: 1,
    });
    expect(signerClasses(tipJar.steps.close!)).toEqual(["wallet", "attestation"]);
    expect(signerClasses(tipJar.steps.poke!)).toEqual(["keeper", "attestation"]);
  });
});
