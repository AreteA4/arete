import { memo, useMemo } from "react";
import {
  Background,
  Controls,
  Handle,
  MarkerType,
  Position,
  ReactFlow,
  type Edge,
  type Node,
  type NodeProps,
} from "@xyflow/react";

import type { Graph, GraphNode } from "../graph";
import type { Layout } from "../layout";

export interface DiagramHighlights {
  /** Node and edge ids a rebuild added or changed. */
  changed: Set<string>;
  /** Ids animating right now (a live transition, step or event). */
  active: Set<string>;
  /** Ids that just failed (a failed step's edges). */
  failed: Set<string>;
  /** The selected instance's path. */
  path: Set<string>;
  /** The selected instance's current state node. */
  current: string | null;
  /** The selected step. */
  selectedStep: string | null;
}

export interface FlowDiagramProps {
  graph: Graph;
  layout: Layout;
  counts: Record<string, number>;
  highlights: DiagramHighlights;
  onSelectStep(step: string): void;
  onSelectNode?(node: GraphNode): void;
}

type DiagramNodeData = {
  node: GraphNode;
  count: number;
  classes: string;
};

const hidden = { opacity: 0, width: 1, height: 1, minWidth: 1, minHeight: 1, border: 0 };

function Handles() {
  return (
    <>
      <Handle id="l" type="target" position={Position.Left} style={hidden} isConnectable={false} />
      <Handle id="r" type="source" position={Position.Right} style={hidden} isConnectable={false} />
      <Handle id="t" type="target" position={Position.Top} style={hidden} isConnectable={false} />
      <Handle id="b" type="source" position={Position.Bottom} style={hidden} isConnectable={false} />
    </>
  );
}

const StateNode = memo(function StateNode({ data }: NodeProps<Node<DiagramNodeData>>) {
  const { node, count, classes } = data;
  return (
    <div className={`node state ${classes}`} data-testid={`node-${node.id}`} title={node.data.description ?? node.label}>
      <Handles />
      <div className="state-name">
        {node.data.initial ? <span className="badge initial">initial</span> : null}
        {node.label}
      </div>
      <div className="state-meta">
        {node.data.derived ? <span className="tag">derived</span> : <span className="tag">recorded</span>}
        {node.data.terminal ? <span className="tag terminal">terminal</span> : null}
        <span className={`count ${count > 0 ? "nonzero" : ""}`} data-testid={`count-${node.id}`}>
          {count}
        </span>
      </div>
    </div>
  );
});

const StartNode = memo(function StartNode({ data }: NodeProps<Node<DiagramNodeData>>) {
  return (
    <div className={`node start ${data.classes}`} title="start" data-testid={`node-${data.node.id}`}>
      <Handles />
    </div>
  );
});

const SideNode = memo(function SideNode({ data }: NodeProps<Node<DiagramNodeData>>) {
  const { node, classes } = data;
  return (
    <div className={`node ${node.kind} ${classes}`} data-testid={`node-${node.id}`} title={node.label}>
      <Handles />
      <span className="side-kind">{node.kind === "reaction" ? "reaction" : node.kind === "event" ? "event" : "step"}</span>
      <span className="side-label">{node.label}</span>
    </div>
  );
});

const LaneNode = memo(function LaneNode({ data }: NodeProps<Node<DiagramNodeData>>) {
  return (
    <div className="lane-box">
      <span className="lane-label">{data.node.label} machine</span>
    </div>
  );
});

const nodeTypes = {
  state: StateNode,
  start: StartNode,
  reaction: SideNode,
  event: SideNode,
  step: SideNode,
  lane: LaneNode,
};

function classesFor(id: string, highlights: DiagramHighlights): string {
  const classes: string[] = [];
  if (highlights.changed.has(id)) classes.push("changed");
  if (highlights.active.has(id)) classes.push("active");
  if (highlights.failed.has(id)) classes.push("failed");
  if (highlights.path.has(id)) classes.push("on-path");
  if (highlights.current === id) classes.push("current");
  return classes.join(" ");
}

export function FlowDiagram({ graph, layout, counts, highlights, onSelectStep, onSelectNode }: FlowDiagramProps) {
  const nodes = useMemo<Node<DiagramNodeData>[]>(() => {
    const laneNodes: Node<DiagramNodeData>[] = layout.lanes.map(({ lane, box }) => ({
      id: `lane:${lane}`,
      type: "lane",
      position: { x: box.x - 16, y: box.y },
      zIndex: -1,
      data: { node: { id: `lane:${lane}`, kind: "state", lane: "side", label: lane, data: {} }, count: 0, classes: "" },
      style: { width: box.width + 32, height: box.height },
      selectable: false,
      draggable: false,
    }));
    const graphNodes = graph.nodes
      .filter((node) => layout.nodes[node.id])
      .map((node) => {
        const box = layout.nodes[node.id]!;
        const isStep = node.kind === "step" && node.data.step === highlights.selectedStep;
        return {
          id: node.id,
          type: node.kind,
          position: { x: box.x, y: box.y },
          data: {
            node,
            count: counts[node.id] ?? 0,
            classes: `${classesFor(node.id, highlights)}${isStep ? " selected-step" : ""}${node.data.derived ? " derived" : ""}`,
          },
          style: { width: box.width, height: box.height },
          draggable: false,
          connectable: false,
        };
      });
    return [...laneNodes, ...graphNodes];
  }, [graph, layout, counts, highlights]);

  const edges = useMemo<Edge[]>(
    () =>
      graph.edges.map((edge) => {
        const classes = classesFor(edge.id, highlights);
        const selected = edge.data.step !== undefined && edge.data.step === highlights.selectedStep;
        const stroke = highlights.failed.has(edge.id)
          ? "var(--fail)"
          : highlights.active.has(edge.id)
            ? "var(--live)"
            : highlights.changed.has(edge.id)
              ? "var(--changed)"
              : selected || highlights.path.has(edge.id)
                ? "var(--accent)"
                : edge.kind === "step"
                  ? "var(--edge)"
                  : "var(--edge-muted)";
        const vertical = edge.kind === "trigger" || edge.kind === "emit";
        return {
          id: edge.id,
          source: edge.source,
          target: edge.target,
          sourceHandle: vertical ? "b" : "r",
          targetHandle: vertical ? "t" : "l",
          label: edge.kind === "step" || edge.kind === "derive" ? edge.label : undefined,
          animated: highlights.active.has(edge.id) || edge.kind === "derive",
          className: `edge-${edge.kind} ${classes}${selected ? " selected-step" : ""}`,
          style: {
            stroke,
            strokeWidth: selected || highlights.active.has(edge.id) || highlights.path.has(edge.id) ? 2.6 : 1.6,
            strokeDasharray: edge.dashed ? "6 4" : undefined,
          },
          labelStyle: { fill: "var(--text)", fontSize: 11 },
          labelBgStyle: { fill: "var(--panel)", fillOpacity: 0.92 },
          labelBgPadding: [6, 3] as [number, number],
          labelBgBorderRadius: 4,
          markerEnd: { type: MarkerType.ArrowClosed, color: stroke, width: 16, height: 16 },
          data: { step: edge.data.step },
          interactionWidth: 18,
        };
      }),
    [graph, highlights],
  );

  // A rebuild that changes the graph's shape remounts the view, which fits it again.
  const shape = useMemo(() => graph.nodes.map((node) => node.id).join("|"), [graph]);

  return (
    <ReactFlow
      key={shape}
      nodes={nodes}
      edges={edges}
      nodeTypes={nodeTypes}
      fitView
      fitViewOptions={{ padding: 0.12, maxZoom: 1.25 }}
      minZoom={0.2}
      proOptions={{ hideAttribution: true }}
      nodesDraggable={false}
      nodesConnectable={false}
      onEdgeClick={(_, edge) => {
        const step = (edge.data as { step?: string } | undefined)?.step;
        if (step) onSelectStep(step);
      }}
      onNodeClick={(_, node) => {
        const data = node.data as DiagramNodeData | undefined;
        if (!data?.node) return;
        if (data.node.kind === "step" && data.node.data.step) onSelectStep(data.node.data.step);
        onSelectNode?.(data.node);
      }}
    >
      <Background gap={24} size={1} color="var(--grid)" />
      <Controls showInteractive={false} position="bottom-right" />
    </ReactFlow>
  );
}
