import { act, render, screen, waitFor, fireEvent } from "@testing-library/react";
import { getWallets } from "@wallet-standard/app";
import type { Wallet, WalletAccount } from "@wallet-standard/base";
import { afterEach, describe, expect, it, vi } from "vitest";

import { BrowserWalletProvider, useBrowserWallet, WalletBar } from "./BrowserWallet";
import { base64ToBytes, bytesToBase64, chainFor, isUsable, signTransactions } from "./standard";

const ADDRESS = "Wa11et" + "1".repeat(38);

function fakeWallet(name = "Fake Wallet", versions: Array<"legacy" | 0> = ["legacy", 0]) {
  const account: WalletAccount = {
    address: ADDRESS,
    publicKey: new Uint8Array(32).fill(9),
    chains: ["solana:mainnet", "solana:devnet"],
    features: ["solana:signTransaction"],
  };
  const signed: Uint8Array[] = [];
  const wallet = {
    version: "1.0.0",
    name,
    icon: "data:image/svg+xml;base64,PHN2Zy8+",
    chains: ["solana:mainnet", "solana:devnet"],
    accounts: [] as WalletAccount[],
    features: {
      "standard:connect": { version: "1.0.0", connect: async () => ({ accounts: [account] }) },
      "standard:disconnect": { version: "1.0.0", disconnect: async () => undefined },
      "solana:signTransaction": {
        version: "1.0.0",
        supportedTransactionVersions: versions,
        signTransaction: async (...inputs: Array<{ transaction: Uint8Array; chain?: string }>) =>
          inputs.map(({ transaction, chain }) => {
            expect(chain).toBe("solana:mainnet");
            const out = new Uint8Array([...transaction, 0xff]);
            signed.push(out);
            return { signedTransaction: out };
          }),
      },
    },
  } as unknown as Wallet;
  return { wallet, account, signed };
}

afterEach(() => vi.unstubAllGlobals());

describe("Wallet Standard", () => {
  it("accepts wallets that connect and sign v0 transactions", () => {
    expect(isUsable(fakeWallet().wallet)).toBe(true);
    expect(isUsable(fakeWallet("Legacy only", ["legacy"]).wallet)).toBe(false);
    expect(chainFor(fakeWallet().account)).toBe("solana:mainnet");
  });

  it("signs base64 transactions with signTransaction", async () => {
    const { wallet, account } = fakeWallet();
    const out = await signTransactions(wallet, account, [bytesToBase64(new Uint8Array([1, 2]))]);
    expect(Array.from(base64ToBytes(out[0]!))).toEqual([1, 2, 0xff]);
  });

  it("connects, prepares for the wallet, signs in it and submits through a4 dev", async () => {
    const { wallet, signed } = fakeWallet("Provider Wallet");
    const calls: Array<{ url: string; body: any }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string, init?: RequestInit) => {
        const body = init?.body ? JSON.parse(String(init.body)) : undefined;
        calls.push({ url, body });
        if (url === "/dev/v1/wallet/prepare")
          return new Response(JSON.stringify({ state: "prepared", executionId: "e9", instanceId: null, wallet: { pubkey: ADDRESS }, intent: {}, transactions: [{ transactionBase64: bytesToBase64(new Uint8Array([7])) }] }));
        if (url === "/dev/v1/wallet/submit") return new Response(JSON.stringify({ step: "open", state: "completed", executionId: "e9" }));
        return new Response("{}", { status: 404 });
      }),
    );
    let captured: ReturnType<typeof useBrowserWallet> = null;
    function Probe() {
      captured = useBrowserWallet();
      return null;
    }
    const unregister = getWallets().register(wallet);
    render(
      <BrowserWalletProvider>
        <WalletBar />
        <Probe />
      </BrowserWalletProvider>,
    );
    fireEvent.click(await screen.findByText(/Connect Provider Wallet/));
    expect(await screen.findByTestId("connected-wallet")).toHaveTextContent("Wa11");
    const previews: unknown[] = [];
    let result: unknown;
    await act(async () => {
      result = await captured!.run({ step: "open", instance: null, args: { amount: "1" } }, (preview) => previews.push(preview));
    });
    expect(result).toMatchObject({ state: "completed" });
    expect(previews).toHaveLength(1);
    expect(calls[0]).toEqual({ url: "/dev/v1/wallet/prepare", body: { step: "open", instance: null, args: { amount: "1" }, pubkey: ADDRESS } });
    expect(calls[1]).toEqual({ url: "/dev/v1/wallet/submit", body: { executionId: "e9", signedTransactions: [bytesToBase64(new Uint8Array([7, 0xff]))] } });
    expect(signed).toHaveLength(1);
    await waitFor(() => expect(screen.getByTestId("airdrop-connected")).toBeInTheDocument());
    unregister();
  });

  it("shows the local network notice, and forgets the wallet on disconnect", async () => {
    const { wallet } = fakeWallet("Leaving Wallet");
    const disconnected = vi.fn(async () => undefined);
    (wallet.features as any)["standard:disconnect"] = { version: "1.0.0", disconnect: disconnected };
    let captured: ReturnType<typeof useBrowserWallet> = null;
    function Probe() {
      captured = useBrowserWallet();
      return null;
    }
    const unregister = getWallets().register(wallet);
    render(
      <BrowserWalletProvider>
        <WalletBar />
        <Probe />
      </BrowserWalletProvider>,
    );
    expect(await screen.findByTestId("local-network-notice")).toHaveTextContent("cannot land on mainnet");
    fireEvent.click(await screen.findByText(/Connect Leaving Wallet/));
    expect(await screen.findByTestId("connected-wallet")).toBeInTheDocument();
    expect(screen.getByTestId("local-network-notice")).toHaveTextContent("burner wallet");
    fireEvent.click(screen.getByText("Disconnect"));
    await waitFor(() => expect(screen.queryByTestId("connected-wallet")).toBeNull());
    expect(disconnected).toHaveBeenCalled();
    expect(captured!.wallet).toBeNull();
    await expect(captured!.run({ step: "open", instance: null, args: {} }, () => undefined)).rejects.toThrow("Connect a wallet first");
    unregister();
  });

  it("surfaces a4 dev refusing a transaction that could land on mainnet", async () => {
    const { wallet, signed } = fakeWallet("Careful Wallet");
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        new Response(JSON.stringify({ error: "Refused to hand transaction 0 to the browser wallet: not a Surfpool synthetic blockhash", code: "dev-ui-not-local" }), {
          status: 409,
        }),
      ),
    );
    let captured: ReturnType<typeof useBrowserWallet> = null;
    function Probe() {
      captured = useBrowserWallet();
      return null;
    }
    const unregister = getWallets().register(wallet);
    render(
      <BrowserWalletProvider>
        <WalletBar />
        <Probe />
      </BrowserWalletProvider>,
    );
    fireEvent.click(await screen.findByText(/Connect Careful Wallet/));
    await screen.findByTestId("connected-wallet");
    await expect(captured!.run({ step: "open", instance: null, args: {} }, () => undefined)).rejects.toThrow(/synthetic blockhash/);
    expect(signed).toHaveLength(0);
    unregister();
  });
});
