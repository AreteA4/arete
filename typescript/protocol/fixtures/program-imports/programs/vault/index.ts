/**
 * A stand-in for the program module `a4 install program vault --ts`
 * generates, with only the properties a protocol reads.
 */
export const VAULT_INSTRUCTION_SHAPES = {
  withdraw: {
    args: ["amount"],
    accounts: {
      vault: { optional: false, derive: ["owner"] },
      owner: { optional: false, derive: null },
      destination: { optional: false, derive: null },
      token_program: { optional: false, derive: [] },
    },
  },
} as const;

export const VAULT_PROGRAM_INSTALL = {
  alias: "vault",
  programReleaseHash: "arete:h1:program-release:sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
  source: { kind: "registry", registry: "vault", version: "1.2.3" },
} as const;

export const VAULT_PROGRAM = {
  name: "vault",
  programId: "Vau1t11111111111111111111111111111111111111",
  programSpecHash: "arete:h1:program-spec:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
  programInstall: VAULT_PROGRAM_INSTALL,
  instructionShapes: VAULT_INSTRUCTION_SHAPES,
} as const;

export default VAULT_PROGRAM;
