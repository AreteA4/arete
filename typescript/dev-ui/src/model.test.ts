import { describe, expect, it } from "vitest";

import { snapshot } from "./fixtures";
import { activeInstances, executionsOf, initialModel, reduce, type Model } from "./model";
import type { DevEvent } from "./types";

let seq = 0;
const event = (type: string, data: Record<string, unknown>): DevEvent => ({
  seq: ++seq,
  type,
  at: new Date(1_800_000_000_000 + seq * 1000).toISOString(),
  data,
});

function fold(model: Model, ...events: DevEvent[]): Model {
  return events.reduce((current, next) => reduce(current, { type: "event", event: next, now: 1000 }), model);
}

describe("reduce", () => {
  const ready = reduce(reduce(initialModel, { type: "snapshot", snapshot: snapshot() }), { type: "live" });

  it("tracks instances of the active deployment from transitions", () => {
    const model = fold(
      ready,
      event("instance.transitioned", { instanceId: "i1", deploymentId: "d1", machine: "instance", from: null, to: "open", cause: { kind: "step", ref: "open" } }),
      event("instance.transitioned", { instanceId: "i2", deploymentId: "d0", machine: "instance", from: null, to: "open" }),
      event("instance.transitioned", { instanceId: "i1", deploymentId: "d1", machine: "instance", from: "open", to: "closed", cause: { kind: "step", ref: "close" } }),
      event("instance.emitted", { instanceId: "i1", event: "tipJar.closed", reaction: "announceClosed" }),
    );
    expect(activeInstances(model).map((instance) => [instance.id, instance.state])).toEqual([["i1", "closed"]]);
    const history = model.instances.i1!.history.map((entry) => entry.label);
    expect(history).toEqual(["∅ → open (step open)", "open → closed (step close)", "emitted tipJar.closed (announceClosed)"]);
    expect(model.pulses.map((pulse) => pulse.kind)).toEqual(["emit", "transition", "transition", "transition"]);
  });

  it("lists history from before the session without animating it", () => {
    const model = fold(
      ready,
      event("instance.transitioned", { instanceId: "i1", deploymentId: "d1", machine: "instance", from: null, to: "open", history: true }),
      event("instance.emitted", { instanceId: "i1", event: "tipJar.opened", reaction: "r", history: true }),
      event("step.confirmed", { executionId: "e0", step: "open", state: "completed", instanceId: "i1", history: true }),
    );
    expect(model.instances.i1!.history).toHaveLength(3);
    expect(model.pulses).toEqual([]);
    const live = fold(model, event("instance.transitioned", { instanceId: "i1", deploymentId: "d1", machine: "instance", from: "open", to: "closed" }));
    expect(live.pulses.map((pulse) => pulse.kind)).toEqual(["transition"]);
  });

  it("folds step events into executions, including rejections", () => {
    const model = fold(
      ready,
      event("step.prepared", { executionId: "e1", step: "close", state: "prepared", instanceId: "i1", wallet: "W" }),
      event("step.submitted", { executionId: "e1", step: "close", state: "submitting" }),
      event("step.failed", { executionId: "e1", step: "close", state: "chain_failed", transactions: [{ index: 0, state: "failed", error: "0x1" }], error: "0x1" }),
      event("step.failed", { executionId: null, step: "close", state: "rejected", rejected: "not_recipient", instanceId: "i1", source: "history" }),
    );
    const executions = executionsOf(model, "close");
    expect(executions).toHaveLength(2);
    expect(executions[0]!.rejected).toBe("not_recipient");
    expect(executions[1]!.state).toBe("chain_failed");
    expect(executions[1]!.updates.map((update) => update.type)).toEqual(["step.prepared", "step.submitted", "step.failed"]);
    expect(executions[1]!.wallet).toBe("W");
    expect(model.instances.i1!.history.map((entry) => entry.ok)).toEqual([false, false]);
  });

  it("does not animate replayed events and skips duplicates", () => {
    const replaying = reduce(initialModel, { type: "snapshot", snapshot: snapshot() });
    const first = event("instance.transitioned", { instanceId: "i9", deploymentId: "d1", machine: "instance", from: null, to: "open" });
    const model = fold(replaying, first, first);
    expect(model.pulses).toHaveLength(0);
    expect(model.instances.i9!.history).toHaveLength(1);
    expect(model.log).toHaveLength(1);
  });

  it("asks for a reload after a rebuild, a deploy or a reset, and tracks build state", () => {
    let model = fold(ready, event("build.started", { changed: ["protocol.ts"] }));
    expect(model.snapshot?.build.state).toBe("building");
    model = fold(model, event("build.failed", { error: "boom", diagnostics: [{ severity: "error", message: "boom" }] }));
    expect(model.snapshot?.build).toMatchObject({ state: "failed", diagnostics: [{ message: "boom" }] });
    expect(model.snapshot?.spec?.name).toBe("tipJar");
    expect(model.needsReload).toBe(false);
    expect(fold(model, event("build.succeeded", {})).needsReload).toBe(true);
    expect(fold(model, event("deploy.activated", {})).needsReload).toBe(true);
    const reset = fold(model, { seq: 0, type: "stream.reset", at: "", data: {} });
    expect(reset.needsReload).toBe(true);
    expect(reset.lastSeq).toBe(0);
  });

  it("keeps the chain clock and merges listed instances", () => {
    let model = fold(ready, { seq: 0, type: "chain.clock", at: "", data: { slot: 5, unixTimestamp: 1700000000 } });
    expect(model.chain).toEqual({ slot: 5, unixTimestamp: 1700000000 });
    expect(model.log).toHaveLength(0);
    model = reduce(model, { type: "instances", instances: [{ id: "i7", deploymentId: "d1", state: "open" }] });
    expect(activeInstances(model).map((instance) => instance.id)).toContain("i7");
  });
});
