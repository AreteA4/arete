import { useCallback, useEffect, useMemo, useReducer, useState } from "react";

import { fetchInstances, fetchProtocol, subscribe } from "./api";
import { BuildOverlay } from "./components/BuildOverlay";
import { EventLog } from "./components/EventLog";
import { FlowDiagram, type DiagramHighlights } from "./components/FlowDiagram";
import { InstancePanel } from "./components/InstancePanel";
import { StatusBar } from "./components/StatusBar";
import { StepPanel, type StepDraft } from "./components/StepPanel";
import {
  buildGraph,
  changedIds,
  diffSummary,
  edgesForTransition,
  eventId,
  reactionId,
  stateCounts,
  stateId,
  type Graph,
} from "./graph";
import { layoutGraph, type Layout } from "./layout";
import { activeInstances, executionsOf, initialModel, reduce, type Model } from "./model";

/** How long a live change glows on the diagram. */
const PULSE_MS = 2600;

/** Ids to animate for the pulses still glowing at `now`. */
export function activeHighlights(model: Model, graph: Graph, now: number) {
  const active = new Set<string>();
  const failed = new Set<string>();
  for (const pulse of model.pulses) {
    if (now - pulse.at > PULSE_MS) continue;
    if (pulse.kind === "transition") {
      for (const id of edgesForTransition(graph, pulse.machine, pulse.from, pulse.to, pulse.cause)) active.add(id);
      active.add(stateId(pulse.machine, pulse.to));
    } else if (pulse.kind === "emit") {
      active.add(eventId(pulse.event));
      if (pulse.reaction) active.add(reactionId(pulse.reaction));
    } else if (!pulse.ok) {
      for (const edge of graph.edges) if (edge.data.step === pulse.step) failed.add(edge.id);
      failed.add(`step:${pulse.step}`);
    }
  }
  return { active, failed };
}

/** The selected instance's path: the states it visited and the edges it took. */
export function instancePath(model: Model, graph: Graph, instanceId: string | null) {
  const path = new Set<string>();
  const instance = instanceId ? model.instances[instanceId] : undefined;
  if (!instance) return { path, current: null };
  for (const entry of instance.history) {
    if (entry.kind !== "transition" || !entry.machine || !entry.to) continue;
    path.add(stateId(entry.machine, entry.to));
    for (const id of edgesForTransition(graph, entry.machine, entry.from ?? null, entry.to, entry.cause ?? null)) {
      path.add(id);
    }
  }
  return { path, current: instance.state ? stateId("instance", instance.state) : null };
}

export interface AppProps {
  /**
   * Extra panel content for a selected step (the run form). It reports its
   * choices through `onDraft` so the panel's copied command follows them.
   */
  renderStepActions?(
    step: string,
    instanceId: string | null,
    model: Model,
    onDraft: (draft: StepDraft) => void,
  ): React.ReactNode;
  /** Extra toolbar content (dev tools). */
  renderTools?(model: Model): React.ReactNode;
}

export function App({ renderStepActions, renderTools }: AppProps = {}) {
  const [model, dispatch] = useReducer(reduce, initialModel);
  const [selectedStep, setSelectedStep] = useState<string | null>(null);
  const [selectedInstance, setSelectedInstance] = useState<string | null>(null);
  const [draft, setDraft] = useState<StepDraft | null>(null);
  const [layout, setLayout] = useState<{ graph: Graph; layout: Layout } | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const [loadError, setLoadError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      dispatch({ type: "snapshot", snapshot: await fetchProtocol() });
      setLoadError(null);
    } catch (error) {
      setLoadError((error as Error).message);
    }
    try {
      dispatch({ type: "instances", instances: (await fetchInstances()).instances });
    } catch {
      // No deployment yet: the counts start at zero.
    }
  }, []);

  useEffect(() => {
    void load();
    return subscribe({
      onEvent: (event) => dispatch({ type: "event", event }),
      onLive: () => dispatch({ type: "live" }),
      onStatus: (connection) => {
        dispatch({ type: "connection", connection });
        // A new a4 dev run: everything may have changed.
        if (connection === "live") void load();
      },
    });
  }, [load]);

  useEffect(() => {
    if (!model.needsReload) return;
    dispatch({ type: "reloaded" });
    void load();
  }, [model.needsReload, load]);

  const spec = model.snapshot?.spec ?? null;
  const machineMap = model.snapshot?.machineMap ?? null;
  const graph = useMemo(() => (spec ? buildGraph(spec, machineMap) : null), [spec, machineMap]);

  useEffect(() => {
    if (!graph) return;
    let cancelled = false;
    layoutGraph(graph).then(
      (next) => {
        if (!cancelled) setLayout({ graph, layout: next });
      },
      () => undefined,
    );
    return () => {
      cancelled = true;
    };
  }, [graph]);

  // Re-render while something glows.
  const glowing = model.pulses.some((pulse) => Date.now() - pulse.at < PULSE_MS);
  useEffect(() => {
    if (!glowing) return;
    const timer = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(timer);
  }, [glowing, model.pulses]);

  const instances = activeInstances(model);
  const counts = useMemo(() => stateCounts(instances), [instances]);
  const build = model.snapshot?.build ?? null;

  const highlights = useMemo<DiagramHighlights | null>(() => {
    if (!layout) return null;
    const { active, failed } = activeHighlights(model, layout.graph, now);
    const { path, current } = instancePath(model, layout.graph, selectedInstance);
    return {
      changed: changedIds(layout.graph, build?.state === "ok" ? build.diff : null),
      active,
      failed,
      path,
      current,
      selectedStep,
    };
  }, [layout, model, now, selectedInstance, selectedStep, build]);

  const step = selectedStep && spec ? spec.steps[selectedStep] : undefined;
  const title = spec ? `${spec.name}@${spec.version}` : (model.snapshot?.project ?? "a4 dev");

  return (
    <div className="app">
      <StatusBar
        title={title}
        connection={model.connection}
        build={build}
        buildSummary={build?.state === "ok" ? diffSummary(build.diff) : ""}
        deployment={model.snapshot?.deployment ?? null}
        environment={model.snapshot?.environment ?? null}
        chain={model.chain}
        degraded={model.degraded}
      />
      {renderTools ? <div className="toolbar">{renderTools(model)}</div> : null}
      <main className="main">
        <section className="diagram" data-testid="diagram">
          {layout && highlights ? (
            <FlowDiagram
              graph={layout.graph}
              layout={layout.layout}
              counts={counts}
              highlights={highlights}
              onSelectStep={(name) => setSelectedStep(name)}
            />
          ) : (
            <div className="empty">
              {loadError ? `Cannot reach a4 dev: ${loadError}` : spec ? "Laying out…" : "Waiting for the first build…"}
            </div>
          )}
          {build ? <BuildOverlay build={build} /> : null}
        </section>
        <aside className="side">
          {step && selectedStep ? (
            <StepPanel
              name={selectedStep}
              step={step}
              slots={model.snapshot?.anatomy?.steps[selectedStep]?.instructions}
              executions={executionsOf(model, selectedStep)}
              instanceId={selectedInstance}
              walletNames={Object.fromEntries((model.snapshot?.wallets ?? []).map((wallet) => [wallet.pubkey, wallet.name]))}
              draft={draft}
              onClose={() => setSelectedStep(null)}
            >
              {renderStepActions?.(selectedStep, selectedInstance, model, setDraft)}
            </StepPanel>
          ) : (
            <>
              <div className="steps-list">
                <h3>Steps</h3>
                <ul>
                  {Object.entries(spec?.steps ?? {}).map(([name, candidate]) => (
                    <li key={name}>
                      <button className="link" onClick={() => setSelectedStep(name)} data-testid={`step-${name}`}>
                        {name}
                      </button>
                      <span className="muted small"> {candidate.scope}</span>
                    </li>
                  ))}
                </ul>
              </div>
              <InstancePanel instances={instances} selected={selectedInstance} onSelect={setSelectedInstance} />
              <EventLog events={model.log} />
            </>
          )}
        </aside>
      </main>
    </div>
  );
}
