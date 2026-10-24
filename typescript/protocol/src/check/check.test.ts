import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { parseContractJson, validateProtocolSpec, type ProtocolSpecV1 } from "../index.js";
import { checkExpression, checkSpecExpressions, type ExpressionScope } from "./expressions.js";
import { checkHandlerSource } from "./static.js";

const here = dirname(fileURLToPath(import.meta.url));
const load = (name: string) =>
  validateProtocolSpec(parseContractJson(readFileSync(resolve(here, `../../../../test-vectors/protocol/${name}`))));

describe("expression reference check", () => {
  it("accepts every expression in the shared fixtures", () => {
    expect(checkSpecExpressions(load("token-raise.protocol-spec.json"))).toEqual([]);
    expect(checkSpecExpressions(load("escrow-resolution.protocol-spec.json"))).toEqual([]);
  });

  it("reports unknown references with spans", () => {
    const spec = load("token-raise.protocol-spec.json") as ProtocolSpecV1;
    spec.steps.contribute!.guards[2]!.expr = "(reads.escorw.totalDeposit ?? 0) + args.amount <= params.walletCap";
    spec.steps.claim!.guards[0]!.expr = 'instance.state == "finalized" && ephemeral.mint != wallet';
    spec.steps.create!.guards[0]!.expr = "args.softCap <= instance.accounts.presale";
    spec.machines.instance.derive[0]!.when = "now >= params.deadlin || foo(1)";
    expect(checkSpecExpressions(spec)).toEqual([
      {
        code: "unknown-reference",
        path: "/machines/instance/derive/0/when",
        offset: 7,
        length: 14,
        message: "param 'deadlin' is not declared here",
      },
      { code: "unknown-function", path: "/machines/instance/derive/0/when", offset: 25, length: 3, message: "unknown function 'foo'" },
      {
        code: "unknown-reference",
        path: "/steps/claim/guards/0/expr",
        offset: 33,
        length: 14,
        message: "ephemeral key 'mint' is not declared here",
      },
      {
        code: "unknown-reference",
        path: "/steps/contribute/guards/2/expr",
        offset: 1,
        length: 25,
        message: "read 'escorw' is not declared here",
      },
      {
        code: "unknown-reference",
        path: "/steps/create/guards/0/expr",
        offset: 16,
        length: 25,
        message: "instance is not in scope here (create steps run before it exists)",
      },
    ]);
  });

  it("requires a Squads step's multisig to come from a resource", () => {
    const spec = load("token-raise.protocol-spec.json") as ProtocolSpecV1;
    const finalize = spec.steps.finalize!;
    if (finalize.authority.kind !== "squadsProposer") throw new Error("finalize is a Squads step");
    const findings = (multisig: string) => {
      if (finalize.authority.kind === "squadsProposer") finalize.authority.multisig = multisig;
      return checkSpecExpressions(spec);
    };
    expect(findings("resources.treasury.multisig")).toEqual([]);
    expect(findings("instance.accounts.multisig")).toEqual([
      {
        code: "squads-multisig-not-from-resources",
        path: "/steps/finalize/authority/multisig",
        offset: 0,
        length: 26,
        message:
          "a Squads step's multisig must come from the deployment config: declare it as a resource and read it with resources.<name>, not 'instance.accounts.multisig'",
      },
    ]);
    expect(findings('if(params.softCap > 0, resources.treasury.multisig, wallet)').map((f) => [f.code, f.offset])).toEqual([
      ["squads-multisig-not-from-resources", 3],
      ["squads-multisig-not-from-resources", 52],
    ]);
    expect(findings("resources.tresury.multisig")).toEqual([
      {
        code: "unknown-reference",
        path: "/steps/finalize/authority/multisig",
        offset: 0,
        length: 26,
        message: "resource 'tresury' is not declared here",
      },
    ]);
    expect(findings('"SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf"')[0]?.code).toBe("squads-multisig-not-from-resources");
    expect(findings("resources")).toEqual([
      {
        code: "unknown-reference",
        path: "/steps/finalize/authority/multisig",
        offset: 0,
        length: 9,
        message: "name a resource: resources.<name>",
      },
    ]);
  });

  it("checks remaining account expressions like account expressions", () => {
    const spec = load("token-raise.protocol-spec.json") as ProtocolSpecV1;
    const finalize = spec.steps.finalize!.build;
    if (finalize.kind !== "template") throw new Error("finalize is a template");
    finalize.instructions[0]!.remainingAccounts = [
      { pubkey: "instance.accounts.presale", isWritable: true, isSigner: false },
      { pubkey: "params.quoteMnt", isWritable: false, isSigner: false },
    ];
    expect(checkSpecExpressions(spec)).toEqual([
      {
        code: "unknown-reference",
        path: "/steps/finalize/build/instructions/0/remainingAccounts/1/pubkey",
        offset: 0,
        length: 15,
        message: "param 'quoteMnt' is not declared here",
      },
    ]);
  });

  it("accepts the deployment root everywhere, including create steps", () => {
    const spec = load("token-raise.protocol-spec.json") as ProtocolSpecV1;
    spec.steps.create!.guards[0]!.expr = "deployment.squadsProposer != null && deployment.attestation != wallet";
    spec.machines.instance.derive[0]!.when = 'deployment.id != ""';
    expect(checkSpecExpressions(spec)).toEqual([]);
    const scope: ExpressionScope = {
      now: true,
      wallet: false,
      params: new Set(),
      participant: false,
      ephemeral: new Set(),
      reads: new Set(),
      resources: new Set(),
    };
    expect(checkExpression("deployment.proposer", scope)).toEqual([
      {
        code: "unknown-reference",
        path: "",
        offset: 0,
        length: 19,
        message: "deployment exposes id, attestation and squadsProposer",
      },
    ]);
    expect(checkExpression("deployment", scope)[0]?.code).toBe("unknown-reference");
    expect(checkExpression("deployment.id.x", scope)[0]?.code).toBe("unknown-reference");
  });

  it("tokenizes the documented syntax", () => {
    const scope: ExpressionScope = {
      now: true,
      wallet: true,
      params: new Set(["a"]),
      args: new Set(["b"]),
      participant: false,
      ephemeral: new Set(),
      reads: new Set(),
      resources: new Set(),
    };
    expect(
      checkExpression('if(params.a >= 1e3, [1, 2] , "x\\"y") ?? (args.b << 2) % 3 != !true && u64(none) | 1 ^ 2 & 3', scope),
    ).toEqual([]);
    expect(checkExpression("params.a @ 1", scope)).toEqual([
      { code: "invalid-expression", path: "", offset: 9, length: 1, message: "unexpected character '@'" },
    ]);
  });
});

describe("static handler check", () => {
  it("accepts the fixture handler", () => {
    const source = readFileSync(resolve(here, "../../fixtures/token-raise/handlers/createLaunch.ts"), "utf8");
    expect(checkHandlerSource(source, "handlers/createLaunch.ts")).toEqual([]);
  });

  it("rejects network, process, dynamic code and Node built-ins", () => {
    const source = [
      'import { readFileSync } from "node:fs";',
      'import net from "net";',
      'import type { Socket } from "node:net";',
      "// fetch in a comment is fine",
      'const label = "process and fetch in a string are fine";',
      "const pattern = /eval\\(/;",
      "export async function handler() {",
      "  await fetch(`https://example.invalid/${process.env.KEY}`);",
      "  const ws = new WebSocket('wss://example.invalid');",
      "  globalThis.process.exit(1);",
      "  eval('1');",
      "  new Function('return 1');",
      "  await import('./other.js');",
      "  require('fs');",
      "  const x = { fetch: 1, process: 2 };",
      "  return x.fetch + Deno.version + Bun.version;",
      "}",
    ].join("\n");
    const findings = checkHandlerSource(source, "h.ts").map((f) => `${f.code}@${f.line}:${f.column}`);
    expect(findings).toEqual([
      "forbidden-import@1:1",
      "forbidden-import@2:1",
      "forbidden-global@8:9",
      "forbidden-global@8:42",
      "forbidden-global@9:18",
      "forbidden-global@10:14",
      "dynamic-code@11:3",
      "dynamic-code@12:7",
      "dynamic-import@13:9",
      "forbidden-global@14:3",
      "forbidden-global@16:20",
      "forbidden-global@16:35",
    ]);
  });
});
