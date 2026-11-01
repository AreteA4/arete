/**
 * Prints what a client reads from the chain to fill the step args that must
 * match it: `npx tsx scripts/chain-values.ts <rpc url> <plan> <subscriber>`.
 * `RPC_TOKEN` is the bearer token for the RPC, when it needs one.
 *
 * Layouts: `state/plan.rs`, `state/subscription_authority.rs` and
 * `state/subscription_delegation.rs` of the subscriptions program (all packed).
 */
import { getSubscriptionAuthorityPda, getSubscriptionPda } from "../generated/typescript/programs/subscriptions/subscriptions-extensions.js";

const [rpcUrl, plan, subscriber] = process.argv.slice(2);
if (!rpcUrl || !plan || !subscriber) throw new Error("usage: chain-values.ts <rpc url> <plan> <subscriber>");

async function account(address: string): Promise<Buffer | null> {
  const response = await fetch(rpcUrl!, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      ...(process.env.RPC_TOKEN ? { authorization: `Bearer ${process.env.RPC_TOKEN}` } : {}),
    },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "getAccountInfo", params: [address, { encoding: "base64" }] }),
  });
  if (!response.ok) throw new Error(`getAccountInfo ${address}: HTTP ${response.status}`);
  const { result, error } = (await response.json()) as { result?: { value: { data: [string, string] } | null }; error?: unknown };
  if (error) throw new Error(`getAccountInfo ${address}: ${JSON.stringify(error)}`);
  return result?.value ? Buffer.from(result.value.data[0], "base64") : null;
}

const planData = await account(plan);
if (!planData) throw new Error(`no plan at ${plan}`);
const mint = base58(planData.subarray(43, 75));
const authority = await account(getSubscriptionAuthorityPda({ user: subscriber, tokenMint: mint }));
const subscription = await account(getSubscriptionPda({ planPda: plan, subscriber }));

console.log(
  JSON.stringify({
    planId: planData.readBigUInt64LE(35).toString(),
    planBump: planData[33],
    status: planData[34],
    mint,
    amount: planData.readBigUInt64LE(75).toString(),
    periodHours: planData.readBigUInt64LE(83).toString(),
    createdAt: planData.readBigInt64LE(91).toString(),
    endTs: planData.readBigInt64LE(99).toString(),
    pullers: [235, 267, 299, 331].map((at) => base58(planData.subarray(at, at + 32))),
    metadataUri: [...planData.subarray(363, 491)],
    authorityInitId: authority ? authority.readBigInt64LE(98).toString() : null,
    expiresAtTs: subscription ? subscription.readBigInt64LE(147).toString() : null,
  }),
);

function base58(bytes: Uint8Array): string {
  const alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
  let value = BigInt(`0x${Buffer.from(bytes).toString("hex")}`);
  let text = "";
  while (value > 0n) {
    text = alphabet[Number(value % 58n)] + text;
    value /= 58n;
  }
  for (const byte of bytes) {
    if (byte !== 0) break;
    text = `1${text}`;
  }
  return text;
}
