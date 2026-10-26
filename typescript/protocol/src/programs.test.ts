import { cpSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { buildProgramSpecV1FromBytes, hashProgramSpecV1 } from "@usearete/hash";
import { afterAll, describe, expect, it } from "vitest";

import tipJar from "../fixtures/program-imports/programs/tip-jar/index.js";
import vault from "../fixtures/program-imports/programs/vault/index.js";
import { buildProtocol, checkProtocol, writeBuild } from "./build/build.js";
import { parseLockDependencies } from "./build/lock.js";
import {
  ProtocolDefinitionError,
  builtins,
  defineProtocol,
  isInstalledProgram,
  ix,
  lowerProtocol,
  parseContractJson,
  type ProtocolDefinition,
} from "./index.js";

const here = dirname(fileURLToPath(import.meta.url));
const fixture = resolve(here, "../fixtures/program-imports");
const temporary: string[] = [];

afterAll(() => {
  for (const directory of temporary) rmSync(directory, { recursive: true, force: true });
});

function copyFixture(): string {
  const directory = mkdtempSync(join(tmpdir(), "arete-protocol-programs-"));
  temporary.push(directory);
  cpSync(fixture, directory, { recursive: true });
  return directory;
}

const base = {
  name: "tips",
  version: "0.1.0",
  instanceVisibility: "shared",
  machines: { instance: { initial: "open", states: { open: "recorded", done: { terminal: true } } } },
} as const;

function define(steps: ProtocolDefinition["steps"], programs: ProtocolDefinition["programs"] = { tipJar, vault }) {
  return defineProtocol({
    ...base,
    programs,
    steps: {
      open: {
        scope: "create",
        authority: "creator",
        bind: { accounts: { owner: "wallet" } },
        build: [ix(tipJar, "tip", { args: { amount: "1" }, accounts: { owner: "wallet", tipper: "wallet" } })],
      },
      close: {
        scope: "instance",
        authority: "keeper",
        transitions: [{ to: "done" }],
        build: [ix(tipJar, "tip", { args: { amount: "1" }, accounts: { owner: "wallet", tipper: "wallet" } })],
      },
      ...steps,
    },
  });
}

describe("installed programs", () => {
  it("are recognised by their install shape", () => {
    expect(isInstalledProgram(tipJar)).toBe(true);
    expect(isInstalledProgram({ ...vault })).toBe(true);
    expect(isInstalledProgram({ programId: vault.programId, programSpecHash: vault.programSpecHash })).toBe(false);
    expect(isInstalledProgram({ ...vault, programInstall: { alias: "vault", source: { kind: "path" } } })).toBe(false);
  });

  it("lower to their pins, with the release of a registry install", () => {
    const lowered = lowerProtocol(define({}, { tipJar, vault, ...builtins("system", "splToken") }));
    expect(lowered.spec.programs).toEqual({
      tipJar: { kind: "programSpec", programId: tipJar.programId, programSpecHash: tipJar.programSpecHash },
      vault: {
        kind: "programSpec",
        programId: vault.programId,
        programSpecHash: vault.programSpecHash,
        programReleaseHash: vault.programInstall.programReleaseHash,
      },
      system: { kind: "builtin", builtin: "system" },
      splToken: { kind: "builtin", builtin: "splToken" },
    });
    expect(lowered.programSources).toEqual({
      tipJar: { idl: "./idl/tip_jar.json" },
      vault: { registry: "vault", version: "=1.2.3" },
    });
    expect(Object.keys(lowered.installedPrograms)).toEqual(["tipJar", "vault"]);
  });

  it("pin the ProgramSpec hash built from the installed IDL", () => {
    const bytes = readFileSync(join(fixture, tipJar.programInstall.source.idl));
    expect(hashProgramSpecV1(buildProgramSpecV1FromBytes(bytes))).toBe(tipJar.programSpecHash);
  });

  it("builtins() returns typed modules and rejects names that are not builtins", () => {
    const { associatedToken } = builtins("associatedToken");
    expect(associatedToken.builtin).toBe("associatedToken");
    expect(associatedToken.programId).toBe("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
    expect(associatedToken.programSpecHash).toMatch(/^arete:h1:program-spec:sha256:/);
    expect(Object.keys(associatedToken.instructionShapes)).toContain("createIdempotent");
    expect(isInstalledProgram(associatedToken)).toBe(false);
    // @ts-expect-error not a builtin
    expect(() => builtins("vault")).toThrow(/not a builtin/);
    // @ts-expect-error the registry name of a builtin
    expect(() => builtins("spl-ata")).toThrow("'spl-ata' is the builtin 'associatedToken'");
    // @ts-expect-error the program SDK name of a builtin
    expect(() => builtins("splAta")).toThrow("'splAta' is the builtin 'associatedToken'");
  });

  it("refuses the removed stackSource option", () => {
    const definition = { ...define({}), stackSource: { programs: {} } };
    expect(() => lowerProtocol(definition)).toThrow(expect.objectContaining({ path: "/stackSource" }));
  });
});

describe("ix", () => {
  it("lowers to a template with the program's alias in `programs`", () => {
    const { spec } = lowerProtocol(
      define(
        {
          tip: {
            scope: "instance",
            authority: "keeper",
            args: { amount: "u64" },
            build: [
              ix(tipJar, "tip", {
                args: { amount: "args.amount" },
                accounts: { owner: "instance.accounts.owner", tipper: "wallet" },
                remainingAccounts: [{ pubkey: "instance.accounts.owner" }],
              }),
            ],
          },
        },
        { jar: tipJar },
      ),
    );
    const build = spec.steps.tip!.build;
    expect(build).toEqual({
      kind: "template",
      instructions: [
        {
          program: "jar",
          instruction: "tip",
          args: { amount: "args.amount" },
          accounts: { owner: "instance.accounts.owner", tipper: "wallet" },
          remainingAccounts: [{ pubkey: "instance.accounts.owner", isWritable: false, isSigner: false }],
        },
      ],
    });
  });

  it("checks names at compile time", () => {
    const accounts = { owner: "instance.accounts.owner", tipper: "wallet" };
    // @ts-expect-error unknown instruction
    ix(tipJar, "withdraw", { args: { amount: "1" }, accounts });
    // @ts-expect-error unknown argument
    ix(tipJar, "tip", { args: { amount: "1", memo: "x" }, accounts });
    // @ts-expect-error missing argument
    ix(tipJar, "tip", { args: {}, accounts });
    // @ts-expect-error missing required account (tipper)
    ix(tipJar, "tip", { args: { amount: "1" }, accounts: { owner: "wallet" } });
    // @ts-expect-error unknown account
    ix(tipJar, "tip", { args: { amount: "1" }, accounts: { ...accounts, payer: "wallet" } });
    // Optional and derivable accounts may be given or left out.
    ix(tipJar, "tip", { args: { amount: "1" }, accounts: { ...accounts, referrer: "wallet", jar: "wallet" } });
    expect(true).toBe(true);
  });

  it("checks builtin instructions like installed ones (builtins-ata)", () => {
    const { associatedToken, system } = builtins("associatedToken", "system");
    const accounts = {
      fundingAddress: "wallet",
      associatedTokenAccountAddress: 'ata(wallet, "So11111111111111111111111111111111111111112")',
      walletAddress: "wallet",
      tokenMintAddress: '"So11111111111111111111111111111111111111112"',
      systemProgram: '"11111111111111111111111111111111"',
      tokenProgram: '"TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"',
    };
    // @ts-expect-error unknown account (the IDL names it `walletAddress`)
    ix(associatedToken, "createIdempotent", { accounts: { ...accounts, wallet: "wallet" } });
    const { walletAddress: _, ...withoutWallet } = accounts;
    // @ts-expect-error missing required account (walletAddress)
    ix(associatedToken, "createIdempotent", { accounts: withoutWallet });
    // @ts-expect-error unknown instruction
    ix(associatedToken, "createIdempotentt", { accounts });
    // @ts-expect-error missing argument (lamports)
    ix(system, "transfer", { accounts: { fundingAccount: "wallet", recipientAccount: "wallet" } });

    const programs = { tipJar, associatedToken, system };
    const { spec } = lowerProtocol(
      define(
        {
          prepare: {
            scope: "instance",
            authority: "keeper",
            build: [
              ix(associatedToken, "createIdempotent", { accounts }),
              ix(system, "transfer", {
                args: { lamports: "1" },
                accounts: { fundingAccount: "wallet", recipientAccount: "instance.accounts.owner" },
              }),
            ],
          },
        },
        programs,
      ),
    );
    expect(spec.programs.associatedToken).toEqual({ kind: "builtin", builtin: "associatedToken" });
    expect(spec.steps.prepare!.build).toEqual({
      kind: "template",
      instructions: [
        { program: "associatedToken", instruction: "createIdempotent", args: {}, accounts },
        {
          program: "system",
          instruction: "transfer",
          args: { lamports: "1" },
          accounts: { fundingAccount: "wallet", recipientAccount: "instance.accounts.owner" },
        },
      ],
    });

    // Plain JavaScript, or a builtin called by alias: checked when defined.
    const loose = ix as unknown as (program: unknown, name: string, input: unknown) => ReturnType<typeof ix>;
    const step = (instruction: unknown) => ({
      prepare: { scope: "instance" as const, authority: "keeper" as const, build: [instruction as ReturnType<typeof ix>] },
    });
    expect(() =>
      lowerProtocol(define(step(loose(associatedToken, "createIdempotent", { accounts: { ...accounts, wallet: "wallet" } })), programs)),
    ).toThrow(
      expect.objectContaining({
        path: "/steps/prepare/build/0/accounts/wallet",
        message: expect.stringContaining("instruction 'createIdempotent' has no account 'wallet'"),
      }),
    );
    expect(() =>
      lowerProtocol(
        define(
          step({ program: "ata", instruction: "createIdempotent", accounts: withoutWallet }),
          { tipJar, ata: { builtin: "associatedToken" } },
        ),
      ),
    ).toThrow(
      expect.objectContaining({
        path: "/steps/prepare/build/0/accounts",
        message: expect.stringContaining("account 'walletAddress' of 'createIdempotent' is neither supplied nor derivable"),
      }),
    );
  });

  it("checks names against the runtime shapes when the protocol is defined", () => {
    const step = (instruction: ReturnType<typeof ix>) => ({
      tip: { scope: "instance" as const, authority: "keeper" as const, build: [instruction] },
    });
    // A module from plain JavaScript or a stale declaration: types can't help.
    const loose = ix as unknown as (program: unknown, name: string, input: unknown) => ReturnType<typeof ix>;
    const cases: Array<[ReturnType<typeof ix>, string, RegExp]> = [
      [loose(tipJar, "tipp", { args: { amount: "1" } }), "/steps/tip/build/0/instruction", /no instruction 'tipp'/],
      [loose(tipJar, "tip", { args: {} }), "/steps/tip/build/0/args", /argument 'amount'/],
      [
        loose(tipJar, "tip", { args: { amount: "1" }, accounts: { tipper: "wallet", owner: "wallet", nope: "wallet" } }),
        "/steps/tip/build/0/accounts/nope",
        /no account 'nope'/,
      ],
      // `jar` derives from `owner`, which is neither supplied nor derivable.
      [loose(tipJar, "tip", { args: { amount: "1" }, accounts: { tipper: "wallet" } }), "/steps/tip/build/0/accounts", /account 'jar'/],
    ];
    for (const [instruction, path, message] of cases) {
      let error: unknown;
      try {
        lowerProtocol(define(step(instruction)));
      } catch (caught) {
        error = caught;
      }
      expect(error).toBeInstanceOf(ProtocolDefinitionError);
      expect((error as ProtocolDefinitionError).path).toBe(path);
      expect((error as Error).message).toMatch(message);
    }
    // String templates naming an installed program's alias are checked too,
    // with the same name matching as the server (case and `_` ignored).
    expect(() =>
      lowerProtocol(define(step({ program: "tipJar", ix: "Tip", args: { Amount: "1" }, accounts: { owner: "wallet" } } as never))),
    ).toThrow(/account 'tipper'/);
  });

  it("requires the program to be declared in `programs`", () => {
    const instruction = ix(vault, "withdraw", {
      args: { amount: "1" },
      accounts: { owner: "wallet", destination: "wallet" },
    });
    expect(() =>
      lowerProtocol(define({ w: { scope: "instance", authority: "keeper", build: [instruction] } }, { tipJar })),
    ).toThrow(expect.objectContaining({ path: "/steps/w/build/0/program" }));
  });
});

describe("arete-protocol build with installed programs", () => {
  const entry = join(fixture, "protocol.ts");

  it("derives the stack's program sources and copies the installed IDL beside it", async () => {
    const result = await buildProtocol({ entry });
    expect(result.warnings).toEqual([]);
    const stack = parseContractJson(result.files.get("protocol-stack-source.json")!) as {
      stackSource: { programs: Record<string, unknown> };
    };
    expect(stack.stackSource.programs.tipJar).toEqual({
      source: { idl: "./idl/tip_jar.json" },
      program_id: tipJar.programId,
    });
    expect(stack.stackSource.programs.vault).toEqual({ source: { registry: "vault" }, version: "=1.2.3" });
    expect(Buffer.from(result.files.get("idl/tip_jar.json")!)).toEqual(readFileSync(join(fixture, "idl/tip_jar.json")));
    const release = parseContractJson(result.files.get("protocol-release.json")!) as {
      dependencies: { programs: unknown[] };
    };
    expect(release.dependencies.programs).toEqual([
      { alias: "tipJar", programId: tipJar.programId, programSpecHash: tipJar.programSpecHash },
      {
        alias: "vault",
        programId: vault.programId,
        programSpecHash: vault.programSpecHash,
        programReleaseHash: vault.programInstall.programReleaseHash,
      },
    ]);
    const out = mkdtempSync(join(tmpdir(), "arete-protocol-programs-out-"));
    temporary.push(out);
    writeFileSync(join(out, "stale"), "");
    writeBuild(result, out);
    expect(readFileSync(join(out, "idl/tip_jar.json"))).toEqual(readFileSync(join(fixture, "idl/tip_jar.json")));
  });

  it("fails when a program no longer matches arete.lock, naming it and the fix", async () => {
    const project = copyFixture();
    const lock = join(project, "arete.lock");
    writeFileSync(lock, readFileSync(lock, "utf8").replace(tipJar.programSpecHash, `arete:h1:program-spec:sha256:${"0".repeat(64)}`));
    await expect(checkProtocol({ entry: join(project, "protocol.ts") })).rejects.toMatchObject({
      message: "installed programs do not match arete.lock",
      findings: [{ message: expect.stringMatching(/program 'tipJar' \(arete.toml 'tip-jar'\).*a4 install/) }],
    });

    writeFileSync(lock, readFileSync(join(fixture, "arete.lock"), "utf8").replace(/program_release_hash = .*\n/, ""));
    await expect(checkProtocol({ entry: join(project, "protocol.ts") })).rejects.toMatchObject({
      findings: [{ message: expect.stringMatching(/'vault' was generated from release .* arete.lock pins none/) }],
    });
  });

  it("checks and builds a builtin called with ix through the bundled definition", async () => {
    const project = copyFixture();
    const entry = join(project, "protocol.ts");
    const source = readFileSync(entry, "utf8");
    const withBuiltin = source
      .replace(
        'programs: { tipJar, vault, ...builtins("system", "splToken") },',
        'programs: { tipJar, vault, ...programsBuiltins },',
      )
      .replace(
        "export default defineProtocol({",
        'const programsBuiltins = builtins("system", "splToken");\n\nexport default defineProtocol({',
      )
      .replace(
        "        ix(vault, \"withdraw\", {",
        '        ix(programsBuiltins.system, "transfer", {\n          args: { lamports: "1" },\n          accounts: { fundingAccount: "wallet", recipientAccount: "instance.accounts.payout" },\n        }),\n        ix(vault, "withdraw", {',
      );
    expect(withBuiltin).toContain('ix(programsBuiltins.system, "transfer"');
    writeFileSync(entry, withBuiltin);
    const result = await checkProtocol({ entry });
    expect(result.lowered.spec.steps.close!.build).toMatchObject({
      kind: "template",
      instructions: [{ program: "system", instruction: "transfer" }, { program: "vault", instruction: "withdraw" }],
    });
    await expect(buildProtocol({ entry })).resolves.toBeDefined();

    writeFileSync(entry, withBuiltin.replace('fundingAccount: "wallet"', 'funder: "wallet"'));
    await expect(checkProtocol({ entry })).rejects.toMatchObject({
      path: "/steps/close/build/0/accounts/funder",
      message: expect.stringContaining("instruction 'transfer' has no account 'funder'"),
    });
  });

  it("warns and checks nothing without arete.lock", async () => {
    const project = copyFixture();
    rmSync(join(project, "arete.lock"));
    const result = await checkProtocol({ entry: join(project, "protocol.ts") });
    expect(result.warnings).toEqual([expect.stringMatching(/no arete.lock/)]);
  });

  it("fails when an installed IDL is missing", async () => {
    const project = copyFixture();
    rmSync(join(project, "idl/tip_jar.json"));
    expect(existsSync(join(project, "idl/tip_jar.json"))).toBe(false);
    await expect(buildProtocol({ entry: join(project, "protocol.ts") })).rejects.toThrow(/idl\/tip_jar.json, which is missing/);
  });
});

describe("parseLockDependencies", () => {
  it("reads only the string fields of top-level dependency tables", () => {
    const text = [
      "lock_version = 1",
      "",
      "[[dependency]]",
      'kind = "stack"',
      'alias = "ore"',
      "targets = [",
      '    "typescript",',
      "]",
      "",
      "[[dependency.programs]]",
      'program_spec_hash = "nested"',
      "",
      "[[dependency]]",
      "kind = 'program'",
      'alias = "vote"',
      'program_spec_hash = "arete:h1:program-spec:sha256:\\u0031"',
    ].join("\n");
    expect(parseLockDependencies(text)).toEqual([
      { kind: "stack", alias: "ore" },
      { kind: "program", alias: "vote", program_spec_hash: "arete:h1:program-spec:sha256:1" },
    ]);
  });
});
