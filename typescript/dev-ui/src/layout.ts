/**
 * Graph → positions with ELK's layered algorithm, one lane per machine
 * (left to right), lanes stacked top to bottom. Reactions and events sit in
 * the lane of the machine that triggers them.
 */

import ELK from "elkjs/lib/elk.bundled.js";
import type { ElkNode } from "elkjs/lib/elk-api";

import type { Graph, GraphNode } from "./graph";

export interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface Layout {
  nodes: Record<string, Box>;
  lanes: Array<{ lane: string; box: Box }>;
}

const SIZES: Record<GraphNode["kind"], { width: number; height: number }> = {
  state: { width: 176, height: 60 },
  start: { width: 28, height: 28 },
  reaction: { width: 220, height: 44 },
  event: { width: 180, height: 36 },
  step: { width: 210, height: 40 },
};

const LANE_GAP = 56;
const LANE_PADDING = 28;

export function sizeOf(node: GraphNode) {
  return SIZES[node.kind];
}

/** Which lane a node is drawn in. */
export function laneOf(graph: Graph, node: GraphNode): string {
  if (node.lane !== "side") return node.lane;
  // A reaction goes with the machine whose states trigger it; an event with its reaction.
  const incoming = graph.edges.find((edge) => edge.target === node.id);
  if (!incoming) return "instance";
  const source = graph.nodes.find((candidate) => candidate.id === incoming.source);
  return source ? laneOf(graph, source) : "instance";
}

let elk: InstanceType<typeof ELK> | null = null;

const SIDE_GAP_Y = 64;
const SIDE_GAP_X = 36;

export async function layoutGraph(graph: Graph): Promise<Layout> {
  elk ??= new ELK();
  const lanes = ["instance", "participant"].filter((lane) =>
    graph.nodes.some((node) => laneOf(graph, node) === lane),
  );
  const result: Layout = { nodes: {}, lanes: [] };
  let top = 0;
  for (const lane of lanes) {
    const members = graph.nodes.filter((node) => laneOf(graph, node) === lane);
    // ELK lays out the machine (states, start, loose steps); reactions and
    // events go in a row beneath, under the states that trigger them.
    const machine = new Set(members.filter((node) => node.lane !== "side").map((node) => node.id));
    const root: ElkNode = {
      id: `lane:${lane}`,
      layoutOptions: {
        "elk.algorithm": "layered",
        "elk.direction": "RIGHT",
        "elk.spacing.nodeNode": "48",
        "elk.layered.spacing.nodeNodeBetweenLayers": "170",
        "elk.layered.spacing.edgeNodeBetweenLayers": "40",
        "elk.layered.nodePlacement.strategy": "BRANDES_KOEPF",
        "elk.padding": `[top=${LANE_PADDING + 12},left=${LANE_PADDING},bottom=${LANE_PADDING},right=${LANE_PADDING}]`,
      },
      children: members.filter((node) => machine.has(node.id)).map((node) => ({ id: node.id, ...sizeOf(node) })),
      edges: graph.edges
        .filter((edge) => machine.has(edge.source) && machine.has(edge.target) && edge.source !== edge.target)
        .map((edge) => ({ id: edge.id, sources: [edge.source], targets: [edge.target] })),
    };
    const laid = await elk.layout(root);
    let right = laid.width ?? 0;
    let bottom = 0;
    for (const child of laid.children ?? []) {
      const box = { x: child.x ?? 0, y: top + (child.y ?? 0), width: child.width ?? 0, height: child.height ?? 0 };
      result.nodes[child.id] = box;
      bottom = Math.max(bottom, box.y + box.height);
    }
    if (bottom === 0) bottom = top + LANE_PADDING;
    // Reactions under their first trigger state, then the events they emit.
    const side = members.filter((node) => !machine.has(node.id));
    const rowY = bottom + SIDE_GAP_Y;
    let cursorX = LANE_PADDING;
    let sideBottom = bottom;
    const place = (id: string, x: number, y: number, size: { width: number; height: number }) => {
      result.nodes[id] = { x, y, ...size };
      cursorX = Math.max(cursorX, x + size.width + SIDE_GAP_X);
      right = Math.max(right, x + size.width + LANE_PADDING);
      sideBottom = Math.max(sideBottom, y + size.height);
    };
    for (const node of side.filter((candidate) => candidate.kind === "reaction")) {
      const trigger = graph.edges.find((edge) => edge.target === node.id && result.nodes[edge.source]);
      const anchor = trigger ? result.nodes[trigger.source] : undefined;
      const x = Math.max(cursorX, anchor ? anchor.x : cursorX);
      place(node.id, x, rowY, sizeOf(node));
      for (const emit of graph.edges.filter((edge) => edge.source === node.id && edge.kind === "emit")) {
        const event = side.find((candidate) => candidate.id === emit.target);
        if (event && !result.nodes[event.id]) place(event.id, x, rowY + 44 + SIDE_GAP_Y / 2, sizeOf(event));
      }
    }
    for (const node of side.filter((candidate) => !result.nodes[candidate.id])) {
      place(node.id, cursorX, rowY, sizeOf(node));
    }
    const height = sideBottom - top + LANE_PADDING;
    result.lanes.push({ lane, box: { x: 0, y: top, width: right, height } });
    top += height + LANE_GAP;
  }
  const widest = Math.max(0, ...result.lanes.map((lane) => lane.box.width));
  for (const lane of result.lanes) lane.box.width = widest;
  return result;
}
