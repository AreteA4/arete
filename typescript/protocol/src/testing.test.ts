import { describe, expect, it } from "vitest";
import { type PrepareInputV1, type ProtocolHandlerFnV1 } from "./abi.js";
import { createReplayContext, runHandler, type RecordedCtxCallV1 } from "./testing.js";

const ADDRESS = "11111111111111111111111111111111";
const operation = { abi: "arete.protocol-handler/v1" as const, transactions: [{name:"test",instructions:[{programId:ADDRESS,keys:[],dataBase64:""}]}] };
const swallow: ProtocolHandlerFnV1 = async (_input, ctx) => {
  try { await ctx.solana.account(ADDRESS); } catch { /* The replay must still reject. */ }
  return operation;
};

describe("sticky replay failures", () => {
  it("rejects an extra call caught by the handler with an empty recording", async () => {
    await expect(runHandler(swallow, {} as PrepareInputV1, [])).rejects.toMatchObject({code:"ctx-replay-divergence"});
  });
  it("keeps a method mismatch even if the caller later tries the recorded call", async () => {
    const ctx = createReplayContext([{method:"stacks.view",args:{stack:"s",view:"v",key:"k"},result:null}]);
    await expect(ctx.solana.account(ADDRESS)).rejects.toMatchObject({code:"ctx-replay-divergence"});
    await expect(ctx.stacks.view("s","v","k")).rejects.toMatchObject({code:"ctx-replay-divergence"});
    expect(()=>ctx.assertConsumed()).toThrow(/expected stacks.view/);
  });
  it("retains result-integrity failures after they are caught", async () => {
    const calls: RecordedCtxCallV1[] = [{method:"solana.account",args:{pubkey:ADDRESS},result:null,resultSha256:"0".repeat(64)}];
    await expect(runHandler(swallow,{} as PrepareInputV1,calls)).rejects.toMatchObject({code:"ctx-replay-result-mismatch"});
  });
  it("rejects a caught extra call after consuming the entire recording", async () => {
    const ctx = createReplayContext([{method:"solana.account",args:{pubkey:ADDRESS},result:null}]);
    await ctx.solana.account(ADDRESS);
    await expect(ctx.solana.account(ADDRESS)).rejects.toMatchObject({code:"ctx-replay-divergence"});
    expect(()=>ctx.assertConsumed()).toThrow(/was not recorded/);
  });
});
