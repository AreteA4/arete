import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { StepPanel, type StepDraft } from "../components/StepPanel";
import { tipJar } from "../fixtures";
import { RunStepForm } from "./RunStepForm";

const WALLETS = {
  wallets: [
    { name: "creator", pubkey: "C".repeat(43) + "1", lamports: 5e9 },
    { name: "recipient", pubkey: "R".repeat(43) + "1", lamports: 7e9 },
  ],
};

function mockServer() {
  const calls: Array<{ url: string; body: any }> = [];
  const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
    const body = init?.body ? JSON.parse(String(init.body)) : undefined;
    calls.push({ url, body });
    const reply = (value: unknown) => new Response(JSON.stringify(value), { status: 200 });
    if (url === "/dev/v1/wallets") return reply(WALLETS);
    if (url === "/dev/v1/instances/i1") return reply({ id: "i1", state: "open", roles: { recipient: "R".repeat(43) + "1" } });
    if (url === "/dev/v1/steps/prepare")
      return reply({
        state: "prepared",
        executionId: "e1",
        instanceId: "i1",
        wallet: { name: body.wallet, pubkey: "R".repeat(43) + "1" },
        intent: {},
        transactions: [
          {
            version: "v0",
            size: 400,
            signatures: [false, true],
            requiredSignatures: 2,
            accounts: [{ index: 0, pubkey: "R".repeat(43) + "1", signer: true, writable: true }],
            instructions: [{ programId: "11111111111111111111111111111111", accounts: [], dataLength: 12, dataBase64: "" }],
            anatomy: { instructions: [{ role: "program", kind: "handler" }] },
            simulation: { err: null, logs: ["Program log: ok"] },
          },
        ],
      });
    if (url === "/dev/v1/steps/run") return reply({ step: "close", state: "completed", executionId: "e2" });
    if (url === "/dev/v1/instances/i2") return reply({ id: "i2", state: "open", roles: {} });
    if (url === "/dev/v1/steps/submit")
      return reply({ step: "close", state: "completed", executionId: "e1", instanceId: "i1", transactions: [{ index: 0, state: "confirmed", signature: "S1g" }] });
    return new Response("{}", { status: 404 });
  });
  vi.stubGlobal("fetch", fetchMock);
  (globalThis as any).__fallback = fetchMock;
  return calls;
}

afterEach(() => vi.unstubAllGlobals());

describe("RunStepForm", () => {
  it("prepares with the role holder's dev wallet, previews, then submits", async () => {
    const calls = mockServer();
    render(
      <RunStepForm
        name="close"
        step={tipJar.steps.close!}
        instances={[{ id: "i1", deploymentId: "d1", state: "open", history: [] }]}
        selectedInstance="i1"
      />,
    );
    // The recipient role suggests the recipient's dev wallet.
    await waitFor(() => expect((screen.getByTestId("run-as") as HTMLSelectElement).value).toBe("dev:recipient"));
    fireEvent.click(screen.getByTestId("preview-button"));
    expect(await screen.findByText("required")).toBeInTheDocument();
    fireEvent.change(screen.getByPlaceholderText("u64 (a decimal string)"), { target: { value: "1000" } });
    fireEvent.click(screen.getByTestId("preview-button"));
    expect(await screen.findByTestId("preview")).toBeInTheDocument();
    expect(calls.find((call) => call.url === "/dev/v1/steps/prepare")?.body).toEqual({
      step: "close",
      instance: "i1",
      args: { amount: "1000" },
      wallet: "recipient",
    });
    expect(screen.getByText("System")).toBeInTheDocument();
    expect(screen.getByTestId("simulation")).toHaveTextContent("simulation ok");
    fireEvent.click(screen.getByTestId("submit-button"));
    expect(await screen.findByTestId("run-result")).toHaveTextContent("completed");
    expect(calls.find((call) => call.url === "/dev/v1/steps/submit")?.body).toEqual({ executionId: "e1" });
  });

  it("keeps the copied a4 dev step command in step with the form", async () => {
    mockServer();
    function Panel() {
      const [draft, setDraft] = useState<StepDraft | null>(null);
      return (
        <StepPanel name="close" step={tipJar.steps.close!} executions={[]} instanceId="i1" draft={draft} onClose={vi.fn()}>
          <RunStepForm
            name="close"
            step={tipJar.steps.close!}
            instances={[{ id: "i1", deploymentId: "d1", state: "open", history: [] }]}
            selectedInstance="i1"
            onDraft={setDraft}
          />
        </StepPanel>
      );
    }
    render(<Panel />);
    const command = () => screen.getByText(/^a4 dev step close/).textContent;
    // The suggested role holder signs; the empty amount is a placeholder.
    await waitFor(() => expect(command()).toBe(`a4 dev step close --instance i1 --args '{"amount":"<amount>"}' --wallet recipient`));
    fireEvent.change(screen.getByPlaceholderText("u64 (a decimal string)"), { target: { value: "1000" } });
    fireEvent.change(screen.getByTestId("run-as"), { target: { value: "dev:creator" } });
    expect(command()).toBe(`a4 dev step close --instance i1 --args '{"amount":"1000"}' --wallet creator`);
  });

  const INSTANCES = [
    { id: "i1", deploymentId: "d1", state: "open", history: [] },
    { id: "i2", deploymentId: "d1", state: "open", history: [] },
  ];
  const BROWSER = [{ id: "browser", label: "Fake 1234…abcd (browser wallet)" }];

  it("falls back to a dev wallet when the browser wallet disconnects", async () => {
    const calls = mockServer();
    const runWithExtra = vi.fn();
    const props = { name: "close", step: tipJar.steps.close!, instances: INSTANCES, selectedInstance: "i1" };
    const { rerender } = render(<RunStepForm {...props} extraSigners={BROWSER} runWithExtra={runWithExtra} />);
    await waitFor(() => expect((screen.getByTestId("run-as") as HTMLSelectElement).value).toBe("dev:recipient"));
    fireEvent.change(screen.getByTestId("run-as"), { target: { value: "browser" } });
    expect((screen.getByTestId("run-as") as HTMLSelectElement).value).toBe("browser");
    // The wallet disconnects: no browser signer any more.
    rerender(<RunStepForm {...props} extraSigners={[]} />);
    await waitFor(() => expect((screen.getByTestId("run-as") as HTMLSelectElement).value).toBe("dev:recipient"));
    fireEvent.change(screen.getByPlaceholderText("u64 (a decimal string)"), { target: { value: "5" } });
    fireEvent.click(screen.getByTestId("run-button"));
    await waitFor(() => expect(calls.some((call) => call.url === "/dev/v1/steps/run")).toBe(true));
    expect(runWithExtra).not.toHaveBeenCalled();
  });

  it("keeps an explicit browser wallet choice when the instance changes", async () => {
    mockServer();
    const runWithExtra = vi.fn(async () => ({ step: "close", state: "completed" }));
    render(<RunStepForm name="close" step={tipJar.steps.close!} instances={INSTANCES} selectedInstance="i1" extraSigners={BROWSER} runWithExtra={runWithExtra} />);
    await waitFor(() => expect((screen.getByTestId("run-as") as HTMLSelectElement).value).toBe("dev:recipient"));
    fireEvent.change(screen.getByTestId("run-as"), { target: { value: "browser" } });
    fireEvent.change(screen.getByTestId("run-instance"), { target: { value: "i2" } });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect((screen.getByTestId("run-as") as HTMLSelectElement).value).toBe("browser");
    fireEvent.change(screen.getByPlaceholderText("u64 (a decimal string)"), { target: { value: "5" } });
    fireEvent.click(screen.getByTestId("run-button"));
    expect(await screen.findByTestId("run-result")).toHaveTextContent("completed");
    expect(runWithExtra).toHaveBeenCalledWith("browser", { step: "close", instance: "i2", args: { amount: "5" } }, expect.any(Function));
  });

  it("drops the preview when the args change, so a stale one is never signed", async () => {
    mockServer();
    render(<RunStepForm name="close" step={tipJar.steps.close!} instances={INSTANCES} selectedInstance="i1" />);
    await waitFor(() => expect((screen.getByTestId("run-as") as HTMLSelectElement).value).toBe("dev:recipient"));
    fireEvent.change(screen.getByPlaceholderText("u64 (a decimal string)"), { target: { value: "1000" } });
    fireEvent.click(screen.getByTestId("preview-button"));
    expect(await screen.findByTestId("preview")).toBeInTheDocument();
    fireEvent.change(screen.getByPlaceholderText("u64 (a decimal string)"), { target: { value: "2000" } });
    await waitFor(() => expect(screen.queryByTestId("preview")).toBeNull());
    expect(screen.queryByTestId("submit-button")).toBeNull();
  });

  it("shows a terminal rejection's message and hint", async () => {
    const calls = mockServer();
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string, init?: RequestInit) => {
        if (url === "/dev/v1/steps/prepare")
          return new Response(
            JSON.stringify({
              step: "close",
              state: "rejected",
              rejected: "instance_terminal",
              detail: { rejected: "instance_terminal", detail: { message: "the jar is closed" }, hint: "Read protocol_actions first" },
            }),
          );
        return (globalThis as any).__fallback(url, init);
      }),
    );
    void calls;
    render(<RunStepForm name="close" step={tipJar.steps.close!} instances={INSTANCES} selectedInstance="i1" />);
    await waitFor(() => expect((screen.getByTestId("run-as") as HTMLSelectElement).value).toBe("dev:recipient"));
    fireEvent.change(screen.getByPlaceholderText("u64 (a decimal string)"), { target: { value: "1" } });
    fireEvent.click(screen.getByTestId("preview-button"));
    expect(await screen.findByTestId("run-result")).toHaveTextContent("instance_terminal");
    expect(screen.getByTestId("run-rejection")).toHaveTextContent("the jar is closed");
    expect(screen.getByTestId("run-hint")).toHaveTextContent("Read protocol_actions first");
  });
});
