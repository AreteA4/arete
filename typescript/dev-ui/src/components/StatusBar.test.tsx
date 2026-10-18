import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { EventLog } from "./EventLog";
import { StatusBar } from "./StatusBar";

describe("the dev environment", () => {
  it("is named in the status bar", () => {
    render(
      <StatusBar
        title="jar@0.1.0"
        connection="live"
        build={null}
        buildSummary=""
        deployment={null}
        environment={{ id: "denv_01hzx3k8m9q2r4s5t6v7w8x9yz", name: "tip-jar" }}
        chain={null}
      />,
    );
    expect(screen.getByTestId("environment")).toHaveTextContent("env tip-jar");
  });

  it("lists its lifecycle events", () => {
    render(
      <EventLog
        events={[
          { seq: 3, type: "env.changed", at: "2026-10-17T00:00:00Z", data: { kind: "slept", detail: { reason: "idle" } } },
        ]}
      />,
    );
    expect(screen.getByTestId("event-log")).toHaveTextContent("slept (idle)");
  });
});
