/**
 * A protocol with a handler that imports an installed program module.
 * `programs/vote/` is the CLI's golden output for `a4 install program ...
 * --ts`, which imports its runtime from `@usearete/sdk/program`; the
 * handler bundle has to take that module, and the SDK runtime with it,
 * without Node built-ins or the network client.
 */
import { builtins, defineProtocol } from "@usearete/protocol";
import vote from "@programs/vote";

import { ping } from "./handlers/ping.js";

export default defineProtocol({
  name: "votePing",
  version: "0.1.0",
  instanceVisibility: "shared",
  programs: { vote, ...builtins("system") },
  machines: { instance: { initial: "open", states: { open: { terminal: true } } } },
  steps: {
    open: {
      scope: "create",
      authority: "creator",
      handler: ping,
      export: "ping",
      programs: ["vote"],
      bind: { accounts: { payer: "wallet" } },
    },
  },
  stack: { activity: [{ step: "open", program: "vote", instruction: "ping", instanceAccount: "payer", walletAccount: "payer" }] },
});
