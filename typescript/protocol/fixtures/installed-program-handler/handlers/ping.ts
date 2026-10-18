import { wirePreparedOperation, type HostedPreparedOperationV1, type PrepareInputV1 } from "@usearete/protocol";
import { buildInstruction } from "@usearete/sdk/program";
import { pingInstruction } from "@programs/vote";

/** Builds the program's `ping` with the generated builder. */
export async function ping(input: PrepareInputV1): Promise<HostedPreparedOperationV1> {
  const instruction = buildInstruction(pingInstruction, { payer: input.wallet });
  return wirePreparedOperation([{ name: "ping", instructions: [instruction] }], { programs: ["vote"], input });
}
