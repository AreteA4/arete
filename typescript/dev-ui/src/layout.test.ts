import { describe, expect, it } from "vitest";

import { tipJar } from "./fixtures";
import { buildGraph, stateId } from "./graph";
import { layoutGraph } from "./layout";

describe("layoutGraph", () => {
  it("lays out one lane per machine with side nodes beneath their triggers", async () => {
    const graph = buildGraph(tipJar);
    const layout = await layoutGraph(graph);
    expect(layout.lanes.map((lane) => lane.lane)).toEqual(["instance", "participant"]);
    for (const node of graph.nodes) expect(layout.nodes[node.id], node.id).toBeDefined();
    const closed = layout.nodes[stateId("instance", "closed")]!;
    const reaction = layout.nodes["reaction:announceClosed"]!;
    const event = layout.nodes["event:tipJar.closed"]!;
    expect(reaction.y).toBeGreaterThan(closed.y + closed.height);
    expect(event.y).toBeGreaterThan(reaction.y);
    const participant = layout.lanes[1]!.box;
    expect(layout.nodes[stateId("participant", "joined")]!.y).toBeGreaterThanOrEqual(participant.y);
    const open = layout.nodes[stateId("instance", "open")]!;
    expect(closed.x).toBeGreaterThan(open.x);
  });
});
