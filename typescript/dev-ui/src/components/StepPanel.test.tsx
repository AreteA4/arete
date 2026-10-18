import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { tipJar } from "../fixtures";
import type { Execution } from "../model";
import { StepPanel, stepCommand } from "./StepPanel";

const execution: Execution = {
  key: "e1",
  executionId: "e1",
  step: "close",
  state: "chain_failed",
  instanceId: "i1",
  deploymentId: "d1",
  wallet: "Wa11et1111111111111111111111111111111111111",
  args: { amount: "5" },
  transactions: [{ index: 0, state: "failed", signature: "5ig", error: "custom program error: 0x1" }],
  simulation: [{ transaction: 0, err: null, logs: ["Program log: Instruction: Transfer"] }],
  error: "custom program error: 0x1",
  source: "runtime",
  updates: [{ type: "step.failed", state: "chain_failed", at: "2026-10-15T00:00:00Z" }],
  firstAt: "2026-10-15T00:00:00Z",
  lastAt: "2026-10-15T00:00:00Z",
};

describe("StepPanel", () => {
  it("shows params, guards, the template's account mapping and anatomy", () => {
    render(
      <StepPanel
        name="close"
        step={tipJar.steps.close!}
        slots={[
          { role: "runtime", kind: "computeBudget" },
          { role: "program", kind: "template", program: "system", instruction: "transfer" },
          { role: "runtime", kind: "memo" },
        ]}
        executions={[execution]}
        instanceId="i1"
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByText("role recipient")).toBeInTheDocument();
    expect(screen.getByText("wallet == instance.roles.recipient")).toBeInTheDocument();
    expect(screen.getByText("instance.accounts.creator")).toBeInTheDocument();
    expect(screen.getByText("memo (execution id)")).toBeInTheDocument();
    expect(screen.getByText(/a4 dev step close --instance i1/)).toBeInTheDocument();
    fireEvent.click(screen.getByText("chain_failed"));
    expect(screen.getByText(/custom program error: 0x1/, { selector: ".error-text" })).toBeInTheDocument();
    expect(screen.getByText("simulation logs (1)")).toBeInTheDocument();
  });

  it("builds a4 dev step commands", () => {
    expect(stepCommand("open", tipJar.steps.open!)).toBe(
      `a4 dev step open --args '{"recipient":"<pubkey>","amount":"0"}' --wallet default`,
    );
    expect(stepCommand("close", tipJar.steps.close!, { amount: "it's" }, "i1", "bob")).toBe(
      `a4 dev step close --instance i1 --args '{"amount":"it'\\''s"}' --wallet bob`,
    );
  });
});
