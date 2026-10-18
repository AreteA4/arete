/**
 * A one-step protocol whose handler behaves as `args.mode` says. The
 * handler host tests use it to exercise every prepare outcome.
 */
import { defineProtocol } from "@usearete/protocol";

import { probe } from "./handlers/probe.js";

export default defineProtocol({
  name: "handlerProbe",
  version: "0.1.0",
  instanceVisibility: "shared",
  programs: {
    system: { builtin: "system" },
    probeProgram: {
      programId: "Stake11111111111111111111111111111111111111",
      programSpecHash: "arete:h1:program-spec:sha256:d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4",
    },
  },
  machines: { instance: { initial: "open", states: { open: { terminal: true } } } },
  steps: {
    open: {
      scope: "create",
      authority: "creator",
      args: { mode: "string" },
      handler: probe,
      export: "probe",
      programs: ["system"],
      bind: { accounts: { state: "wallet" } },
    },
  },
  stack: { instanceAccounts: { state: { program: "probeProgram", account: "State" } }, activity: [] },
});
