/**
 * A stand-in for the program module `a4 install program ./idl/tip_jar.json
 * --as tip-jar --ts` generates, with only the properties a protocol reads.
 */
export const TIP_JAR_INSTRUCTION_SHAPES = {
  tip: {
    args: ["amount"],
    accounts: {
      jar: { optional: false, derive: ["owner"] },
      owner: { optional: false, derive: null },
      tipper: { optional: false, derive: null },
      referrer: { optional: true, derive: null },
      system_program: { optional: false, derive: [] },
    },
  },
} as const;

export const TIP_JAR_PROGRAM_INSTALL = {
  alias: "tip-jar",
  source: { kind: "idl", idl: "idl/tip_jar.json" },
} as const;

export const TIP_JAR_PROGRAM = {
  name: "tip_jar",
  programId: "Vote111111111111111111111111111111111111111",
  programSpecHash: "arete:h1:program-spec:sha256:7c9129d489592cd48fa2db1d41ed2c3367b4fece52551323b912b4c515c9e6a1",
  programInstall: TIP_JAR_PROGRAM_INSTALL,
  instructionShapes: TIP_JAR_INSTRUCTION_SHAPES,
} as const;

export default TIP_JAR_PROGRAM;
