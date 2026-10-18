/** The a4 dev server's routes that change things (dev wallets sign server-side). */

import { getJson, postJson } from "../api";

export interface DevWalletBalance {
  name: string;
  pubkey: string;
  lamports: number | null;
}

export interface StepInput {
  step: string;
  instance?: string | null;
  args: Record<string, unknown>;
  wallet?: string;
}

/** A prepared step's preview. */
export interface PreparedPreview {
  state: "prepared";
  executionId: string;
  instanceId: string | null;
  wallet: { name?: string; pubkey: string };
  intent: Record<string, unknown>;
  transactions: Array<{
    version?: string;
    size?: number;
    signatures?: boolean[];
    requiredSignatures?: number;
    accounts?: Array<{ index: number; pubkey: string; signer: boolean; writable: boolean }>;
    instructions?: Array<{ programId: string; accounts: string[]; dataLength: number; dataBase64: string }>;
    addressTableLookups?: unknown[];
    anatomy?: any;
    simulation?: { err?: unknown; logs?: string[]; unitsConsumed?: number } | null;
    transactionBase64?: string;
    error?: string;
  }>;
}

/** A step's outcome, as `a4 dev step --json` prints it. */
export interface StepResult {
  step: string;
  state: string;
  executionId?: string;
  instanceId?: string | null;
  rejected?: string;
  /** The rejection: the runtime's `detail` (with its `message`) and, for terminal states, a `hint`. */
  detail?: unknown;
  wallet?: { name?: string; pubkey?: string };
  transactions?: Array<{ index: number; state: string; signature?: string | null; error?: string | null }>;
}

export const WALLETS_CHANGED = "a4-dev-wallets-changed";
const changed = () => window.dispatchEvent(new Event(WALLETS_CHANGED));

export const listWallets = () => getJson<{ wallets: DevWalletBalance[] }>("/dev/v1/wallets");

export async function createWallet(name: string) {
  const wallet = await postJson<DevWalletBalance>("/dev/v1/wallets", { name });
  changed();
  return wallet;
}

export async function airdrop(target: { wallet: string } | { pubkey: string }, sol: number) {
  const result = await postJson<{ pubkey: string; airdropped: number; lamports: number | null }>("/dev/v1/airdrop", { ...target, sol });
  changed();
  return result;
}

export const timeTravel = (to: string) =>
  postJson<{ before: number; unixTimestamp: number }>("/dev/v1/time-travel", { to });

export const prepareStep = (input: StepInput) => postJson<PreparedPreview | StepResult>("/dev/v1/steps/prepare", input);

export const submitStep = (executionId: string) => postJson<StepResult>("/dev/v1/steps/submit", { executionId });

export const runStep = (input: StepInput) => postJson<StepResult>("/dev/v1/steps/run", input);

export function isPrepared(result: PreparedPreview | StepResult): result is PreparedPreview {
  return result.state === "prepared" && "executionId" in result && Array.isArray((result as PreparedPreview).transactions);
}
