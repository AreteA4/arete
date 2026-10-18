import type { PreparedPreview } from "./runApi";

const KNOWN_PROGRAMS: Record<string, string> = {
  "11111111111111111111111111111111": "System",
  ComputeBudget111111111111111111111111111111: "Compute Budget",
  MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr: "Memo",
  TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA: "SPL Token",
  ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL: "Associated Token",
};

function short(key: string): string {
  return key.length > 12 ? `${key.slice(0, 4)}…${key.slice(-4)}` : key;
}

/** A prepared step's transactions: accounts, instructions with their anatomy role, simulation. */
export function TransactionPreview({ preview, names }: { preview: PreparedPreview; names: Record<string, string> }) {
  const label = (key: string) => names[key] ?? KNOWN_PROGRAMS[key] ?? short(key);
  return (
    <div className="preview" data-testid="preview">
      <div className="mono small">execution {preview.executionId}</div>
      {preview.transactions.map((transaction, index) => {
        const roles: Array<{ role?: string; kind?: string }> =
          transaction.anatomy?.instructions ?? transaction.anatomy?.slots ?? [];
        const simulation = transaction.simulation;
        return (
          <div key={index} className="preview-tx">
            <div className="small muted">
              transaction {index} · {transaction.version} · {transaction.size} bytes ·{" "}
              {transaction.signatures?.filter(Boolean).length ?? 0}/{transaction.requiredSignatures} signatures pre-applied
            </div>
            <table className="kv accounts">
              <tbody>
                {(transaction.accounts ?? []).map((account) => (
                  <tr key={account.index}>
                    <td className="mono">{label(account.pubkey)}</td>
                    <td className="small muted">
                      {account.signer ? "signer" : ""} {account.writable ? "writable" : "readonly"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            <ol className="anatomy">
              {(transaction.instructions ?? []).map((instruction, position) => (
                <li key={position} className={`slot ${roles[position]?.role ?? ""}`}>
                  <span className="slot-role">{roles[position]?.role ?? ""}</span>
                  {label(instruction.programId)}
                  <span className="muted small">
                    {" "}
                    {roles[position]?.kind ? `${roles[position]?.kind} · ` : ""}
                    {instruction.accounts.length} accounts · {instruction.dataLength} bytes
                  </span>
                </li>
              ))}
            </ol>
            {simulation ? (
              <div className={simulation.err ? "error-text" : "small"} data-testid="simulation">
                simulation {simulation.err ? `failed: ${JSON.stringify(simulation.err)}` : "ok"}
                {simulation.unitsConsumed ? ` · ${simulation.unitsConsumed} CU` : ""}
              </div>
            ) : null}
            {simulation?.logs?.length ? (
              <details>
                <summary>simulation logs ({simulation.logs.length})</summary>
                <pre className="code logs">{simulation.logs.join("\n")}</pre>
              </details>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
