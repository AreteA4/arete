/**
 * Dev-only controls, on `a4 dev up`'s dev API (proxied at /dev/v1 by
 * vite.config.ts): the dev wallets, funding, and steps run as a dev wallet.
 * App.tsx renders it only when the client targets a dev environment. Never
 * ship it to production: the dev API exists only beside `a4 dev up`.
 */
import { useEffect, useState } from "react";

interface DevWallet {
  readonly name: string;
  readonly pubkey: string;
}

async function devApi<T>(path: string, body?: unknown): Promise<T> {
  const response = await fetch(`/dev/v1/${path}`, {
    method: body === undefined ? "GET" : "POST",
    headers: { "content-type": "application/json" },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  const text = await response.text();
  if (!response.ok) throw new Error(`${path}: ${text}`);
  return (text ? JSON.parse(text) : {}) as T;
}

export default function DevPanel({ onChange }: { onChange: () => void }) {
  const [wallets, setWallets] = useState<DevWallet[]>([]);
  const [log, setLog] = useState<string[]>([]);
  const say = (line: string) => setLog((lines) => [line, ...lines].slice(0, 20));
  const load = () =>
    devApi<{ wallets?: DevWallet[] } | DevWallet[]>("wallets").then(
      (body) => setWallets(Array.isArray(body) ? body : (body.wallets ?? [])),
      (error: Error) => say(error.message),
    );
  useEffect(() => {
    void load();
  }, []);

  const run = async (label: string, work: () => Promise<unknown>) => {
    try {
      say(`${label}: ${JSON.stringify(await work())}`);
      onChange();
      void load();
    } catch (error) {
      say(`${label} failed: ${(error as Error).message}`);
    }
  };
  const [alice, bob] = wallets;

  return (
    <aside style={{ border: "2px dashed orange", padding: "1rem", marginTop: "2rem" }}>
      <h2>Dev environment</h2>
      <button onClick={() => run("new wallet", () => devApi("wallets", { name: `wallet${wallets.length + 1}` }))}>Create a dev wallet</button>
      <ul>
        {wallets.map((wallet) => (
          <li key={wallet.pubkey}>
            {wallet.name} {wallet.pubkey}{" "}
            <button onClick={() => run(`fund ${wallet.name}`, () => devApi("airdrop", { wallet: wallet.name, sol: 5 }))}>Fund 5 SOL</button>
          </li>
        ))}
      </ul>
      {alice && bob && (
        <button
          onClick={() =>
            run(`${alice.name} opens a jar for ${bob.name}`, () =>
              devApi("steps/run", { step: "open", wallet: alice.name, args: { recipient: bob.pubkey, amount: "1000000" } }),
            )
          }
        >
          Open a jar ({alice.name} → {bob.name})
        </button>
      )}
      <pre style={{ whiteSpace: "pre-wrap" }}>{log.join("\n")}</pre>
    </aside>
  );
}
