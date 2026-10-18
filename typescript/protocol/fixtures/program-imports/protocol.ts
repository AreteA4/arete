/**
 * A tip jar whose programs are installed with `a4 install program` and
 * imported, so the definition restates no hash, program id or IDL source.
 */
import { builtins, defineProtocol, ix } from "@usearete/protocol";
import tipJar from "@programs/tip-jar";
import vault from "@programs/vault";

export default defineProtocol({
  name: "tipJar",
  version: "0.1.0",
  programs: { tipJar, vault, ...builtins("system", "splToken") },
  instanceVisibility: "shared",
  roles: { owner: {} },
  machines: { instance: { initial: "open", states: { open: "recorded", closed: { terminal: true } } } },
  steps: {
    open: {
      scope: "create",
      authority: "creator",
      args: { amount: "u64" },
      bind: { accounts: { owner: "wallet", payout: "wallet" }, roles: { owner: "wallet" } },
      // The creator's first tip opens the jar. `jar` derives from `owner`,
      // and `system_program` is a fixed address, so neither is given.
      build: [
        ix(tipJar, "tip", {
          args: { amount: "args.amount" },
          accounts: { owner: "wallet", tipper: "wallet" },
        }),
      ],
    },
    close: {
      scope: "instance",
      authority: { kind: "role", role: "owner" },
      guards: { not_owner: "wallet == instance.roles.owner" },
      transitions: [{ to: "closed" }],
      build: [
        ix(vault, "withdraw", {
          args: { amount: "0" },
          accounts: { owner: "instance.accounts.owner", destination: "instance.accounts.payout" },
        }),
      ],
    },
  },
  stack: {
    activity: [
      { step: "open", program: "tipJar", instruction: "tip", instanceAccount: "owner", walletAccount: "owner", amountArg: "amount" },
      { step: "close", program: "vault", instruction: "withdraw", instanceAccount: "owner", walletAccount: "owner" },
    ],
  },
});
