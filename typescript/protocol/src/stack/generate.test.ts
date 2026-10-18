import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { canonicalizeJcs } from "@usearete/hash";
import { describe, expect, it } from "vitest";

import { parseContractJson, validateContract, validateProtocolSpec } from "../index.js";
import type { ProtocolSpecV1 } from "../contracts/types.js";
import { PACKAGE_VERSION } from "../version.js";
import {
  generateStackSource,
  instanceOwnedAccounts,
  MEMO_PROGRAM_ALIAS,
  StackGenerationError,
  type GenerateStackSourceOptions,
} from "./generate.js";

const here = dirname(fileURLToPath(import.meta.url));
const spec = validateProtocolSpec(
  parseContractJson(readFileSync(resolve(here, "../../../../test-vectors/protocol/token-raise.protocol-spec.json"))),
);
const decoder = new TextDecoder();
const canonical = (value: unknown): string => decoder.decode(canonicalizeJcs(value as never));

const ATTESTATION = "Attest1111111111111111111111111111111111111";
const FULL: GenerateStackSourceOptions = {
  profile: "full",
  attestation: ATTESTATION,
  deploymentShortId: "raise1",
  programs: { meteoraPresale: { idl: "./idl/meteora-presale.idl.json" } },
};

type Entity = {
  primary_key: string | string[];
  scope?: string;
  register_from?: Array<Record<string, string>>;
  fields: Record<string, unknown>;
};
const entities = (source: ReturnType<typeof generateStackSource>): Record<string, Entity> =>
  source.stackSource.entities as Record<string, Entity>;

function withSpec(change: (spec: ProtocolSpecV1) => void): ProtocolSpecV1 {
  const copy = structuredClone(spec);
  change(copy);
  return copy;
}

describe("generateStackSource: interim profile", () => {
  it("matches the token-raise golden file", () => {
    const generated = generateStackSource(spec, {
      programs: { meteoraPresale: { registry: "meteora-presale", version: "=1.1.0" } },
    });
    const golden = JSON.parse(readFileSync(resolve(here, "testdata/token-raise.protocol-stack-source.json"), "utf8"));
    expect(generated).toEqual(golden);
    expect(validateContract("protocol-stack-source", generated)).toMatch(/^arete:h1:protocol-stack-source:sha256:/);
  });

  it("is deterministic", () => {
    expect(canonical(generateStackSource(spec))).toBe(canonical(generateStackSource(structuredClone(spec))));
  });

  it("records the generator identity", () => {
    expect(generateStackSource(spec).generator).toEqual({ name: "@usearete/protocol", version: PACKAGE_VERSION });
  });

  it("keys every entity by one field, which generated SDKs require", () => {
    for (const entity of Object.values(entities(generateStackSource(spec)))) {
      expect(typeof entity.primary_key).toBe("string");
    }
  });

  it("declares a local IDL source with the spec's program id", () => {
    const generated = generateStackSource(spec, {
      programs: { meteoraPresale: { idl: "./idl/meteora-presale.idl.json" } },
    });
    expect(generated.stackSource.programs).toEqual({
      meteoraPresale: {
        source: { idl: "./idl/meteora-presale.idl.json" },
        program_id: "presSVxnf9UU8jMxhgSMqaRwNiT36qeBdNeTRKjTdbj",
      },
    });
  });

  it("refuses participant keys whose accounts differ between steps", () => {
    const split = withSpec((copy) => {
      copy.stack.activity.push({
        step: "refund",
        program: "meteoraPresale",
        instruction: "withdraw",
        instanceAccount: "presale",
        walletAccount: "payer",
      });
    });
    expect(() => generateStackSource(split)).toThrow(/same walletAccount/);
  });

  it("refuses generated names that collide once case folded", () => {
    // The Rust generator asserts the same messages.
    const colliding = withSpec((copy) => {
      const entry = { program: "meteoraPresale", instanceAccount: "presale", walletAccount: "owner" };
      copy.stack.activity = [
        { ...entry, step: "a_b", instruction: "first" },
        { ...entry, step: "aB", instruction: "second" },
      ];
    });
    expect(() => generateStackSource(colliding)).toThrow(
      new StackGenerationError("ProtocolParticipant.activity: generated names 'aB_count' and 'a_b_count' collide"),
    );
    expect(() => generateStackSource(colliding, FULL)).toThrow(
      new StackGenerationError("ProtocolInstance.activity: generated names 'aB_count' and 'a_b_count' collide"),
    );
  });
});

describe("generateStackSource: full profile", () => {
  const full = generateStackSource(spec, FULL);

  it("matches the token-raise golden file", () => {
    const golden = JSON.parse(readFileSync(resolve(here, "testdata/token-raise.full.protocol-stack-source.json"), "utf8"));
    expect(full).toEqual(golden);
    expect(full.profile).toBe("full");
    expect(validateContract("protocol-stack-source", full)).toMatch(/^arete:h1:protocol-stack-source:sha256:/);
  });

  it("is deterministic", () => {
    expect(canonical(generateStackSource(structuredClone(spec), FULL))).toBe(canonical(full));
  });

  it("resolves every join through the index it names (strict lookups)", () => {
    expect((full.stackSource as Record<string, unknown>).lookups).toBe("strict");
    expect((generateStackSource(spec).stackSource as Record<string, unknown>).lookups).toBeUndefined();
  });

  it("decodes the memo through the memo program and gates on the attestation signer", () => {
    expect(full.stackSource.programs).toMatchObject({
      [MEMO_PROGRAM_ALIAS]: {
        source: { idl: "./idl/spl-memo.idl.json" },
        program_id: "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr",
      },
    });
    const attested =
      'starts_with(args.memo, "arete:p2:raise1:") && accounts.signer == "Attest1111111111111111111111111111111111111" && accounts.signer.is_signer';
    const instance = entities(full).ProtocolInstance!;
    expect(instance.primary_key).toBe("id.instance");
    expect(instance.register_from).toEqual([
      {
        instruction: "areteMemo.instructions.memo",
        sibling: "meteoraPresale.instructions.deposit",
        pda_field: "sibling.accounts.presale",
        seed_expr: 'split_part(args.memo, ":", 5)',
        guard: attested,
        lookup_name: "instance_accounts",
      },
    ]);
    const id = (instance.fields.id as { fields: Record<string, { guard: string; expr: string }> }).fields;
    expect(id.instance).toMatchObject({ expr: 'split_part(args.memo, ":", 5)', guard: attested });
    expect(id.scope).toMatchObject({ expr: 'split_part(args.memo, ":", 4)', guard: attested });
  });

  it("splits activity into protocol and direct counts", () => {
    const activity = (entities(full).ProtocolInstance!.fields.activity as { fields: Record<string, unknown> }).fields;
    expect(Object.keys(activity).sort()).toEqual([
      "contribute_amount",
      "contribute_count",
      "contribute_direct_count",
      "contribute_protocol_amount",
      "contribute_protocol_count",
      "last_ix_path",
      "last_signature",
      "last_slot",
      "last_timestamp",
    ]);
    expect(activity.contribute_count).toEqual({
      aggregate: {
        from: ["meteoraPresale.instructions.deposit"],
        op: "count",
        lookup_by: "accounts.presale",
        via: "instance_accounts",
      },
    });
    expect(entities(full).ProtocolActivity!.fields.via).toMatchObject({ expr: '"protocol"' });
  });

  it("sums the attested amount per step next to the amount of all activity", () => {
    const instance = entities(full).ProtocolInstance!;
    const activity = (instance.fields.activity as { fields: Record<string, unknown> }).fields;
    // All activity on the presale, attested or direct.
    expect(activity.contribute_amount).toEqual({
      aggregate: {
        from: ["meteoraPresale.instructions.deposit"],
        op: "sum",
        field: "args.max_amount",
        lookup_by: "accounts.presale",
        via: "instance_accounts",
      },
    });
    // Only deposits in a transaction whose attested memo names `contribute`.
    expect(activity.contribute_protocol_amount).toEqual({
      from: ["meteoraPresale.instructions.deposit.args.max_amount"],
      strategy: "sum",
      lookup_by: "__signature",
      via: "last_contribute_signature",
    });
    expect(instance.fields.last_contribute_signature).toEqual({
      type: "string",
      from: `${MEMO_PROGRAM_ALIAS}.instructions.memo`,
      expr: "__signature",
      strategy: "last_write",
      guard: expect.stringContaining('split_part(args.memo, ":", 7) == "contribute"'),
      emit: false,
    });
    expect((instance as Entity & { indexes: string[] }).indexes).toEqual(["id.instance", "last_contribute_signature"]);
  });

  it("scopes every entity of an account-visibility protocol from the attested memo", () => {
    expect(spec.instanceVisibility).toBe("account");
    expect(entities(full).ProtocolInstance!.scope).toBe("id.scope");
    expect(entities(full).ProtocolActivity!.scope).toBe("scope");
    expect(entities(full).ProtocolParticipant!.scope).toBe("scope");
  });

  it("declares no scope for a shared protocol", () => {
    const shared = generateStackSource(
      withSpec((copy) => {
        copy.instanceVisibility = "shared";
      }),
      FULL,
    );
    for (const entity of Object.values(entities(shared))) expect(entity.scope).toBeUndefined();
    expect(JSON.stringify(shared)).not.toContain('":", 4)');
  });

  it("gives each attestation key and deployment its own stack", () => {
    const rotated = generateStackSource(spec, { ...FULL, attestation: "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf" });
    const moved = generateStackSource(spec, { ...FULL, deploymentShortId: "raise2" });
    const anyDeployment = generateStackSource(spec, { ...FULL, deploymentShortId: undefined });
    const all = [full, rotated, moved, anyDeployment].map(canonical);
    expect(new Set(all).size).toBe(4);
    expect(JSON.stringify(anyDeployment)).toContain('starts_with(args.memo, \\"arete:p2:\\")');
  });

  it("needs a valid attestation key and deployment short id", () => {
    expect(() => generateStackSource(spec, { ...FULL, attestation: undefined })).toThrow(StackGenerationError);
    expect(() => generateStackSource(spec, { ...FULL, attestation: "not-a-key" })).toThrow(/attestation/);
    expect(() => generateStackSource(spec, { ...FULL, deploymentShortId: 'x") || true || ("' })).toThrow(
      /deploymentShortId/,
    );
  });

  it("reserves the memo program alias", () => {
    const clash = withSpec((copy) => {
      copy.programs[MEMO_PROGRAM_ALIAS] = copy.programs.meteoraPresale!;
      copy.stack.activity[0]!.program = MEMO_PROGRAM_ALIAS;
      copy.stack.instanceAccounts.presale!.program = MEMO_PROGRAM_ALIAS;
    });
    expect(() => generateStackSource(clash, FULL)).toThrow(/reserved/);
  });

  it("rejects an unknown profile", () => {
    expect(() => generateStackSource(spec, { profile: "other" as never })).toThrow(/unknown stack profile/);
  });
});

describe("generateStackSource: steps that share an instruction", () => {
  // Two steps that both pay through the same transfer instruction, as a tip
  // jar's `open` and `close` both build `system.transfer`.
  const transfer = { program: "meteoraPresale", instruction: "deposit", instanceAccount: "presale", walletAccount: "owner" };
  const shared = withSpec((copy) => {
    copy.stack.activity = [
      { ...transfer, step: "contribute", amountArg: "max_amount" },
      { ...transfer, step: "refund", amountArg: "max_amount" },
    ];
  });
  type Field = { from: string | string[]; via?: string };
  const activityFields = (entity: Entity): Record<string, Field> =>
    (entity.fields.activity as { fields: Record<string, Field> }).fields;

  it("reads every instruction once in the attested activity row", () => {
    // Listing the instruction once per step summed each transfer twice.
    const activity = entities(generateStackSource(shared, FULL)).ProtocolActivity!.fields;
    expect((activity.amount as Field).from).toEqual(["meteoraPresale.instructions.deposit.args.max_amount"]);
    expect((activity.activity_account as Field).from).toEqual(["meteoraPresale.instructions.deposit.accounts.presale"]);
  });

  it("names the activity account and keeps the memo's internals off the row", () => {
    const activity = entities(generateStackSource(shared, FULL)).ProtocolActivity!.fields;
    expect(activity.activity_account).toMatchObject({ lookup_by: "__signature", via: "id.signature" });
    for (const internal of ["instance_account", "instructions", "ix_path", "part"]) {
      expect(activity).not.toHaveProperty(internal);
    }
  });

  it("counts a participant's amount only under the step the memo names", () => {
    const participant = entities(generateStackSource(shared, FULL)).ProtocolParticipant!;
    const fields = activityFields(participant);
    expect(fields.contribute_amount!.via).toBe("last_contribute_signature");
    expect(fields.refund_amount!.via).toBe("last_refund_signature");
    expect(participant.fields.last_refund_signature).toMatchObject({
      guard: expect.stringContaining('split_part(args.memo, ":", 7) == "refund"'),
      emit: false,
    });
  });

  it("counts an instance's attested amount only under the step the memo names", () => {
    const instance = entities(generateStackSource(shared, FULL)).ProtocolInstance!;
    const fields = activityFields(instance);
    // The amount of all activity stays per group; the attested amount is per step.
    expect(fields.contribute_or_refund_amount).toBeDefined();
    expect(fields.contribute_protocol_amount).toEqual({
      from: ["meteoraPresale.instructions.deposit.args.max_amount"],
      strategy: "sum",
      lookup_by: "__signature",
      via: "last_contribute_signature",
    });
    expect(fields.refund_protocol_amount!.via).toBe("last_refund_signature");
    expect(instance.fields.last_refund_signature).toMatchObject({
      guard: expect.stringContaining('split_part(args.memo, ":", 7) == "refund"'),
      emit: false,
    });
  });

  it("counts instruction-read activity once, under the merged group", () => {
    const interim = entities(generateStackSource(shared));
    expect(Object.keys(activityFields(interim.ProtocolInstance!)).sort()).toEqual([
      "contribute_or_refund_amount",
      "contribute_or_refund_count",
      "contribute_or_refund_last_slot",
    ]);
    expect(Object.keys(activityFields(interim.ProtocolParticipant!)).sort()).toEqual([
      "contribute_or_refund_amount",
      "contribute_or_refund_count",
      "last_slot",
    ]);
    const full = activityFields(entities(generateStackSource(shared, FULL)).ProtocolInstance!);
    expect(Object.keys(full).filter((name) => name.includes("_count")).sort()).toEqual([
      "contribute_or_refund_count",
      "contribute_or_refund_direct_count",
      "contribute_protocol_count",
      "refund_protocol_count",
    ]);
  });

  it("merges steps transitively and keeps unrelated steps apart", () => {
    const chained = withSpec((copy) => {
      copy.stack.activity = [
        { ...transfer, step: "contribute" },
        { ...transfer, step: "refund" },
        { ...transfer, step: "refund", instruction: "withdraw" },
        { ...transfer, step: "finalize", instruction: "withdraw" },
        { ...transfer, step: "claim", instruction: "claim" },
      ];
    });
    const step = entities(generateStackSource(chained)).ProtocolActivity!.fields.step as { fields: object };
    expect(Object.keys(step.fields)).toEqual(["claim", "contribute_or_finalize_or_refund"]);
  });

  it("refuses rules for one instruction that disagree", () => {
    const mismatch = withSpec((copy) => {
      copy.stack.activity = [
        { ...transfer, step: "contribute", amountArg: "max_amount" },
        { ...transfer, step: "refund" },
      ];
    });
    expect(() => generateStackSource(mismatch)).toThrow(
      new StackGenerationError(
        "stack.activity: steps 'contribute' and 'refund' both list meteoraPresale.instructions.deposit, so they must name the same instanceAccount, walletAccount and amountArg",
      ),
    );
  });

  it("refuses a merged name that is also a step", () => {
    // The Rust generator asserts the same message.
    const clash = withSpec((copy) => {
      copy.stack.activity = [
        { ...transfer, step: "a" },
        { ...transfer, step: "b" },
        { ...transfer, step: "a_or_b", instruction: "claim" },
      ];
    });
    expect(() => generateStackSource(clash)).toThrow(
      new StackGenerationError("stack.activity: the merged step name 'a_or_b' is also a step"),
    );
  });
});

describe("generateStackSource: direct activity", () => {
  type Field = Record<string, unknown>;
  const instanceOf = (source: ReturnType<typeof generateStackSource>): Entity => entities(source).ProtocolInstance!;
  const activityOf = (entity: Entity): Record<string, Field> =>
    (entity.fields.activity as { fields: Record<string, Field> }).fields;
  const createBind = (copy: ProtocolSpecV1) => copy.steps.create!.bind!;

  it("treats accounts bound from the create step's ephemeral keys as the instance's own", () => {
    expect([...instanceOwnedAccounts(spec)].sort()).toEqual(["mint", "multisig", "presale", "vault"]);
    const accounts = withSpec((copy) => {
      createBind(copy).accounts = {
        creator: "wallet",
        recipient: "args.recipient",
        config: 'pda("meteoraPresale", "config", params.quoteMint)',
        quoted: 'pda("meteoraPresale", "x", "ephemeral.mint")',
        undeclared: "ephemeral.other",
        nested: "instance.ephemeral.mint",
        fresh: "ephemeral.mint",
        derived: 'pda("meteoraPresale", "presale", ephemeral.presaleBase)',
      };
    });
    expect([...instanceOwnedAccounts(accounts)].sort()).toEqual(["derived", "fresh"]);
  });

  it("matches direct activity on an instance-owned account (token-raise's presale)", () => {
    const instance = instanceOf(generateStackSource(spec, FULL));
    expect(instance.register_from).toEqual([
      expect.objectContaining({ sibling: "meteoraPresale.instructions.deposit", pda_field: "sibling.accounts.presale" }),
    ]);
    const activity = activityOf(instance);
    expect(activity.contribute_direct_count).toBeDefined();
    expect(activity.last_signature).toEqual({
      from: ["meteoraPresale.instructions.deposit.__signature"],
      lookup_by: "accounts.presale",
      via: "instance_accounts",
    });
  });

  // A tip jar: each step is a bare transfer between two wallets, so its
  // instance account is a wallet that transacts with anyone.
  const transfer = { program: "meteoraPresale", instruction: "deposit", walletAccount: "owner", amountArg: "max_amount" };
  const tipJar = (instanceAccount: string) =>
    withSpec((copy) => {
      createBind(copy).accounts.creator = "wallet";
      copy.stack.activity = [
        { ...transfer, step: "contribute", instanceAccount },
        { ...transfer, step: "refund", instanceAccount },
      ];
    });

  for (const [name, account] of [
    ["the signing wallet", "creator"],
    ["an account the create step does not bind", "destination"],
  ] as const) {
    it(`matches no direct activity on ${name}`, () => {
      const generated = generateStackSource(tipJar(account), FULL);
      const instance = instanceOf(generated);
      // No registration, so no unrelated transfer can reach the instance.
      expect(instance.register_from).toBeUndefined();
      expect(instance.fields.state).toBeUndefined();
      const activity = activityOf(instance);
      const attested = (entities(generated).ProtocolActivity!.fields.attested as { guard: string }).guard;
      expect(Object.keys(activity).sort()).toEqual([
        "contribute_protocol_amount",
        "contribute_protocol_count",
        "last_ix_path",
        "last_signature",
        "last_slot",
        "last_timestamp",
        "refund_protocol_amount",
        "refund_protocol_count",
      ]);
      // Each transfer's amount counts once, under the step its attested memo
      // names, joined by signature rather than through the wallet.
      expect(activity.contribute_protocol_amount).toEqual({
        from: ["meteoraPresale.instructions.deposit.args.max_amount"],
        strategy: "sum",
        lookup_by: "__signature",
        via: "last_contribute_signature",
      });
      expect(activity.refund_protocol_amount).toMatchObject({ via: "last_refund_signature" });
      expect(instance.fields.last_refund_signature).toEqual({
        type: "string",
        from: `${MEMO_PROGRAM_ALIAS}.instructions.memo`,
        expr: "__signature",
        strategy: "last_write",
        guard: `${attested} && split_part(args.memo, ":", 7) == "refund"`,
        emit: false,
      });
      expect((instance as Entity & { indexes: string[] }).indexes).toEqual([
        "id.instance",
        "last_contribute_signature",
        "last_refund_signature",
      ]);
      expect(activity.refund_protocol_count).toMatchObject({
        from: `${MEMO_PROGRAM_ALIAS}.instructions.memo`,
        guard: `${attested} && split_part(args.memo, ":", 7) == "refund"`,
      });
      // The last activity is the last attested protocol transaction, which
      // ProtocolActivity lists.
      expect(activity.last_signature).toEqual({
        from: `${MEMO_PROGRAM_ALIAS}.instructions.memo.__signature`,
        guard: attested,
      });
      expect(activity.last_slot).toEqual({
        from: `${MEMO_PROGRAM_ALIAS}.instructions.memo.__slot`,
        strategy: "max",
        guard: attested,
      });
      expect(JSON.stringify(instance)).not.toContain("instance_accounts");
      expect(validateContract("protocol-stack-source", generated)).toMatch(/^arete:h1:protocol-stack-source:/);
    });
  }
});
