/**
 * Modes: `ok` (no ctx calls), `ctx` (one call of every ctx method),
 * `swallow` (catches a failing ctx call and returns anyway), `throw`,
 * `invalid` (an operation with no transactions), `huge` (over the result
 * size bound) and `hang` (never settles).
 */
import {
  PROTOCOL_HANDLER_ABI_V1,
  wirePreparedOperation,
  type HostedPreparedOperationV1,
  type PrepareInputV1,
  type ProtocolContextV1,
} from "@usearete/protocol";

const TOKEN_PROGRAM = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

export async function probe(
  input: PrepareInputV1<{ mode: string }>,
  ctx: ProtocolContextV1,
): Promise<HostedPreparedOperationV1> {
  const mode = input.args.mode;
  const data = [2, 0, 0, 0];
  if (mode === "throw") throw new Error("probe refused");
  if (mode === "hang") return new Promise<HostedPreparedOperationV1>(() => undefined);
  if (mode === "invalid") return { abi: PROTOCOL_HANDLER_ABI_V1, transactions: [] };
  if (mode === "huge") {
    const ix = { programId: input.programs.system!, keys: [], dataBase64: "A".repeat(13_652) };
    const tx = { name: "huge", instructions: Array.from({ length: 64 }, () => ix) };
    return { abi: PROTOCOL_HANDLER_ABI_V1, transactions: Array.from({ length: 8 }, () => tx) };
  }
  if (mode === "swallow") {
    try {
      await ctx.solana.account(input.wallet);
    } catch {
      // Deliberately ignored: the host must still fail the invocation.
    }
  }
  if (mode === "ctx") {
    const account = await ctx.solana.account(input.wallet, (bytes) => bytes.length, { minContextSlot: 5 });
    const many = await ctx.solana.accounts([input.wallet, TOKEN_PROGRAM]);
    const tokens = await ctx.solana.tokenAccountsByOwner(input.wallet, [TOKEN_PROGRAM]);
    const view = await ctx.stacks.view<{ n: number }>("probeStack", "counter", "k1");
    data.push(account ?? 255, many.filter((entry) => entry !== null).length, tokens.length, view?.n ?? 255);
  }
  return wirePreparedOperation(
    [
      {
        name: "open",
        instructions: [
          {
            programId: input.programs.system!,
            keys: [{ pubkey: input.wallet, isSigner: true, isWritable: true }],
            data: Uint8Array.from(data),
          },
        ],
      },
    ],
    { programs: ["system"], input },
  );
}
