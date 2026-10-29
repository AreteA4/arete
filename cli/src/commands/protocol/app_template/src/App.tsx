import { lazy, Suspense, useCallback, useEffect, useState } from "react";
import { useAllowedSteps, useProtocolInstance } from "@usearete/react";

import { client, isDev } from "./arete";
import { connectWallet, signPrepared } from "./wallet";

// Dev only: loaded inside the `isDev` branch, so a production build that
// targets the published client never runs it.
const DevPanel = lazy(() => import("./DevPanel"));

export function App() {
  const [wallet, setWallet] = useState<string | null>(null);
  const [jars, setJars] = useState<{ id: string; state: string }[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const page = await client.instances!.list({});
      setJars(page.instances.map((jar) => ({ id: jar.id, state: String(jar.state) })));
    } catch (failure) {
      setError((failure as Error).message);
    }
  }, []);
  useEffect(() => {
    void refresh();
  }, [refresh]);

  return (
    <main style={{ fontFamily: "system-ui", maxWidth: 720, margin: "2rem auto" }}>
      <h1>{client.definition.name}</h1>
      {wallet ? <p>Wallet: {wallet}</p> : <button onClick={() => connectWallet().then(setWallet, (e) => setError(e.message))}>Connect wallet</button>}
      {error && <p role="alert">{error}</p>}
      <h2>Tip jars</h2>
      <button onClick={() => void refresh()}>Refresh</button>
      <ul>
        {jars.map((jar) => (
          <li key={jar.id}>
            <button onClick={() => setSelected(jar.id)}>{jar.id}</button> {jar.state}
          </li>
        ))}
      </ul>
      {selected && <Jar id={selected} wallet={wallet} onError={setError} />}
      {isDev && (
        <Suspense fallback={null}>
          <DevPanel onChange={() => void refresh()} />
        </Suspense>
      )}
    </main>
  );
}

function Jar({ id, wallet, onError }: { id: string; wallet: string | null; onError: (message: string) => void }) {
  const { instance, isLoading, error } = useProtocolInstance(client, id, wallet ? { wallet } : {});
  const { data: steps } = useAllowedSteps(client, { instance: id, wallet }, { refreshOnEvents: true });
  const [amount, setAmount] = useState("1000000");
  const [status, setStatus] = useState<string | null>(null);

  // `tip` is a public step: the browser prepares it with the publishable
  // key, the wallet signs the exact bytes, and the client submits them.
  const tip = async () => {
    if (!wallet) return;
    try {
      setStatus("preparing");
      const prepared = await client.steps.tip.prepare({ wallet, instance: id, amount }, { transactionFormat: "v0" });
      if (!prepared.ok) return setStatus(`refused: ${JSON.stringify(prepared.rejection)}`);
      setStatus("waiting for the wallet");
      const signed = await signPrepared(prepared.transactions);
      const outcome = await client.submit(prepared.intent, signed);
      setStatus(String((outcome as { state?: string }).state ?? "submitted"));
    } catch (failure) {
      setStatus(null);
      onError((failure as Error).message);
    }
  };

  if (error) return <p role="alert">{error.message}</p>;
  if (isLoading) return <p>Loading…</p>;
  const canTip = steps?.find((step) => step.step === "tip")?.allowed ?? false;
  return (
    <section>
      <h3>Jar {id}</h3>
      <p>
        State: {String(instance?.state)}; recipient: {String(instance?.roles?.recipient)}
      </p>
      <input value={amount} onChange={(event) => setAmount(event.target.value)} aria-label="lamports" />
      <button disabled={!wallet || !canTip} onClick={() => void tip()}>
        Tip
      </button>
      {status && <p>{status}</p>}
    </section>
  );
}
