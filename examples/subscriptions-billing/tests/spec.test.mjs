// Offline checks of the built spec: who may call each step, and what guards it.
// `npm test` builds protocol.ts into a temporary directory and reads the result.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { after, before, describe, it } from "node:test";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const zero = '"11111111111111111111111111111111"';

let out;
let spec;

const guard = (step, code) => spec.steps[step].guards.find((candidate) => candidate.code === code)?.expr;
const instruction = (step) => spec.steps[step].build.instructions[0];

before(() => {
  out = mkdtempSync(join(tmpdir(), "subscriptions-billing-"));
  execFileSync(join(root, "node_modules/.bin/arete-protocol"), ["build", "--out", out], { cwd: root, stdio: "pipe" });
  spec = JSON.parse(readFileSync(join(out, "protocol-spec.json"), "utf8"));
});

after(() => rmSync(out, { recursive: true, force: true }));

describe("one instance per plan", () => {
  it("has createPlan as its only create step", () => {
    const creates = Object.entries(spec.steps).filter(([, step]) => step.scope === "create");
    assert.deepEqual(creates.map(([name]) => name), ["createPlan"]);
  });

  it("binds the plan PDA from the merchant wallet and planId", () => {
    const expected = 'pda("subscriptions", "plan", wallet, args.planId)';
    assert.equal(spec.steps.createPlan.bind.accounts.plan, expected);
    assert.equal(instruction("createPlan").accounts.planPda, expected);
  });

  it("makes the plan's identity and terms params", () => {
    assert.deepEqual(Object.keys(spec.params.properties).sort(), ["amount", "mint", "periodHours", "planId"]);
    assert.ok(guard("createPlan", "plan_data_mismatch").includes("args.planData.planId == args.planId"));
    assert.ok(guard("createPlan", "plan_data_mismatch").includes("args.planData.terms.amount == args.amount"));
  });

  it("is visible to its account", () => {
    assert.equal(spec.instanceVisibility, "account");
  });
});

describe("roles", () => {
  it("has merchant and puller, and no subscriber role", () => {
    assert.deepEqual(Object.keys(spec.roles).sort(), ["merchant", "puller"]);
  });

  it("binds one puller from pullers[0]", () => {
    assert.deepEqual(spec.steps.createPlan.bind.roles, { merchant: "wallet", puller: "args.planData.pullers[0]" });
  });

  it("refuses more than one puller", () => {
    const expr = guard("createPlan", "too_many_pullers");
    for (const index of [1, 2, 3]) assert.ok(expr.includes(`args.planData.pullers[${index}] == ${zero}`));
    assert.ok(!expr.includes("pullers[0]"));
  });
});

describe("step authority and access", () => {
  const expected = {
    createPlan: ["create", { kind: "creator" }, "secret"],
    updatePlan: ["instance", { kind: "role", role: "merchant" }, "secret"],
    collect: ["instance", { kind: "role", role: "merchant" }, "secret"],
    collectAsPuller: ["instance", { kind: "role", role: "puller" }, "secret"],
    initAuthority: ["participant", { kind: "participant" }, "public"],
    subscribe: ["participant", { kind: "participant" }, "public"],
    cancel: ["participant", { kind: "participant" }, "public"],
    resume: ["participant", { kind: "participant" }, "public"],
  };

  it("declares exactly these steps", () => {
    assert.deepEqual(Object.keys(spec.steps).sort(), Object.keys(expected).sort());
  });

  for (const [name, [scope, authority, access]] of Object.entries(expected)) {
    it(`${name} is ${scope}, ${access}`, () => {
      const step = spec.steps[name];
      assert.equal(step.scope, scope);
      assert.deepEqual(step.authority, authority);
      assert.equal(step.access, access);
      assert.equal(step.build.kind, "template");
    });
  }

  it("leaves out the instructions the protocol doesn't offer", () => {
    const used = Object.values(spec.steps).flatMap((step) => step.build.instructions.map((ix) => ix.instruction));
    for (const name of ["deletePlan", "cancelSubscriptionNow", "createFixedDelegation", "createRecurringDelegation"]) {
      assert.ok(!used.includes(name), name);
    }
  });
});

describe("guards", () => {
  it("gates each role step on its role", () => {
    assert.equal(guard("updatePlan", "not_merchant"), "wallet == instance.roles.merchant");
    assert.equal(guard("collect", "not_merchant"), "wallet == instance.roles.merchant");
    assert.equal(guard("collectAsPuller", "not_puller"), "wallet == instance.roles.puller");
  });

  it("also checks the puller against the chain", () => {
    assert.equal(guard("collectAsPuller", "puller_removed"), "wallet in reads.plan.data.pullers");
  });

  for (const step of ["collect", "collectAsPuller"]) {
    it(`${step} pays only the plan's destinations, the plan's amount`, () => {
      assert.ok(guard(step, "destination_not_allowed").includes("args.destination in reads.plan.data.destinations"));
      assert.ok(guard(step, "transfer_mismatch").includes("args.transferData.amount == params.amount"));
      assert.ok(guard(step, "transfer_mismatch").includes("args.transferData.mint == params.mint"));
      assert.ok(guard(step, "transfer_mismatch").includes("args.transferData.delegator == args.subscriber"));
      assert.deepEqual(spec.steps[step].args.required.sort(), ["destination", "subscriber", "transferData"]);
    });
  }

  it("needs no authority before init, and one before subscribe", () => {
    assert.equal(guard("initAuthority", "authority_exists"), "!exists(reads.subscriptionAuthority.initId)");
    assert.equal(guard("subscribe", "no_authority"), "exists(reads.subscriptionAuthority.initId)");
  });

  it("pins what a subscriber approves to the chain", () => {
    const terms = guard("subscribe", "terms_mismatch");
    for (const part of [
      "args.subscribeData.planBump == reads.plan.bump",
      "args.subscribeData.expectedMint == reads.plan.data.mint",
      "args.subscribeData.expectedAmount == reads.plan.data.terms.amount",
      "args.subscribeData.expectedPeriodHours == reads.plan.data.terms.periodHours",
      "args.subscribeData.expectedCreatedAt == reads.plan.data.terms.createdAt",
    ]) {
      assert.ok(terms.includes(part), part);
    }
    assert.equal(
      guard("subscribe", "authority_mismatch"),
      "args.subscribeData.expectedSubscriptionAuthorityInitId == reads.subscriptionAuthority.initId",
    );
    assert.equal(guard("resume", "stale_subscription"), "args.resumeData.expectedExpiresAtTs == reads.subscription.expiresAtTs");
  });
});

describe("updatePlan", () => {
  it("takes no amount, period, mint or destination", () => {
    const fields = Object.keys(spec.steps.updatePlan.args.properties.updatePlanData.properties);
    for (const forbidden of ["amount", "periodHours", "mint", "destination", "destinations"]) {
      assert.ok(!fields.includes(forbidden), forbidden);
    }
    assert.deepEqual(Object.keys(spec.steps.updatePlan.args.properties), ["updatePlanData"]);
  });

  it("keeps the pullers as they are on chain", () => {
    const expr = guard("updatePlan", "pullers_changed");
    for (const index of [0, 1, 2, 3]) {
      assert.ok(expr.includes(`args.updatePlanData.pullers[${index}] == reads.plan.data.pullers[${index}]`));
    }
  });
});

describe("machines", () => {
  it("starts recorded, then derives active and sunset from the plan's status", () => {
    const { instance } = spec.machines;
    assert.equal(instance.initial, "open");
    assert.equal(instance.states.open.kind, "recorded");
    assert.equal(instance.states.active.kind, "derived");
    assert.equal(instance.states.sunset.kind, "derived");
    assert.deepEqual(instance.derive.map((rule) => [rule.to, rule.when]), [
      ["active", "reads.plan.status == 1"],
      ["sunset", "reads.plan.status == 0"],
    ]);
  });

  it("has no terminal state, so cancel and resume stay callable", () => {
    for (const machine of Object.values(spec.machines)) {
      for (const [name, state] of Object.entries(machine.states)) assert.equal(state.terminal, false, name);
    }
  });

  it("moves participants prospect to subscribed, and subscribed and canceled both ways", () => {
    const moves = (step) => spec.steps[step].transitions.map(({ machine, from, to }) => [machine, from, to]);
    assert.deepEqual(moves("subscribe"), [["participant", ["prospect"], "subscribed"]]);
    assert.deepEqual(moves("cancel"), [["participant", ["subscribed"], "canceled"]]);
    assert.deepEqual(moves("resume"), [["participant", ["canceled"], "subscribed"]]);
  });
});

describe("events and activity", () => {
  it("declares no events", () => {
    assert.deepEqual(spec.events, {});
    assert.deepEqual(spec.reactions, []);
  });

  it("keeps collecting out of the subscriber-keyed activity rows", () => {
    assert.deepEqual(spec.stack.activity.map((rule) => rule.step).sort(), ["cancel", "resume", "subscribe"]);
    for (const rule of spec.stack.activity) assert.equal(rule.walletAccount, "subscriber");
  });
});
