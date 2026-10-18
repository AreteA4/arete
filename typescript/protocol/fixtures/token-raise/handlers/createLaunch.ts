/**
 * The token raise's create step: creates the mint, the Meteora presale and
 * the team's Squads multisig in one transaction. A template can't express
 * it because the presale and multisig accounts derive from keys allocated
 * per intent, and the quote mint must be checked first.
 *
 * Instruction data here is illustrative fixture bytes, not real program
 * layouts; a real handler encodes them with the generated program SDKs.
 */
import {
  wirePreparedOperation,
  type HostedPreparedOperationV1,
  type PrepareInputV1,
  type ProtocolContextV1,
} from "@usearete/protocol";

interface CreateArgs {
  deadline: string;
  softCap: string;
  hardCap: string;
  walletCap: string;
  quoteMint: string;
  members: string[];
}

const PROGRAMS = ["meteoraPresale", "splToken", "squadsV4", "system"] as const;

function u64le(value: string): number[] {
  let remaining = BigInt(value);
  const out: number[] = [];
  for (let i = 0; i < 8; i += 1) {
    out.push(Number(remaining & 0xffn));
    remaining >>= 8n;
  }
  return out;
}

export async function createLaunch(
  input: PrepareInputV1<CreateArgs>,
  ctx: ProtocolContextV1,
): Promise<HostedPreparedOperationV1> {
  const quoteMint = await ctx.solana.account(input.args.quoteMint);
  if (quoteMint === null) throw new Error("quote mint does not exist");

  const mint = input.ephemeral.mint!;
  const presaleBase = input.ephemeral.presaleBase!;
  const createKey = input.ephemeral.createKey!;
  const program = (alias: (typeof PROGRAMS)[number]) => input.programs[alias]!;

  return wirePreparedOperation(
    [
      {
        name: "createLaunch",
        instructions: [
          {
            programId: program("system"),
            keys: [
              { pubkey: input.wallet, isSigner: true, isWritable: true },
              { pubkey: mint, isSigner: true, isWritable: true },
            ],
            data: Uint8Array.from([0, 0, 0, 0]),
          },
          {
            programId: program("splToken"),
            keys: [{ pubkey: mint, isSigner: false, isWritable: true }],
            data: Uint8Array.from([20, 6]),
          },
          {
            programId: program("meteoraPresale"),
            keys: [
              { pubkey: presaleBase, isSigner: true, isWritable: false },
              { pubkey: mint, isSigner: false, isWritable: false },
              { pubkey: input.args.quoteMint, isSigner: false, isWritable: false },
              { pubkey: input.wallet, isSigner: true, isWritable: true },
            ],
            data: Uint8Array.from([
              1,
              ...u64le(input.args.softCap),
              ...u64le(input.args.hardCap),
              ...u64le(input.args.walletCap),
              ...u64le(input.args.deadline),
            ]),
          },
          {
            programId: program("squadsV4"),
            keys: [
              { pubkey: createKey, isSigner: true, isWritable: false },
              { pubkey: input.wallet, isSigner: true, isWritable: true },
              ...input.args.members.map((member) => ({ pubkey: member, isSigner: false, isWritable: false })),
            ],
            data: Uint8Array.from([2, input.args.members.length]),
          },
        ],
      },
    ],
    { programs: PROGRAMS, input },
  );
}
