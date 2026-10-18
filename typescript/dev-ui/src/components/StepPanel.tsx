import { useState, type ReactNode } from "react";

import { authorityLabel, signerClasses } from "../graph";
import type { Execution } from "../model";
import type { AnatomySlot, FieldSchema, Step } from "../types";

export interface StepPanelProps {
  name: string;
  step: Step;
  slots?: AnatomySlot[];
  executions: Execution[];
  /** The instance the copied command targets, for instance steps. */
  instanceId?: string | null;
  /** Names for known wallets (dev wallets), by public key. */
  walletNames?: Record<string, string>;
  /** What the run form holds; the copied command follows it for this step. */
  draft?: StepDraft | null;
  onClose(): void;
  /** Extra content (the run form, when the host can run steps). */
  children?: ReactNode;
}

/** The run form's current choices, for the copied `a4 dev step` command. */
export interface StepDraft {
  /** The step the form is for (a draft for another step is ignored). */
  step: string;
  /** The dev wallet that signs, or null when the signer is not a dev wallet. */
  wallet: string | null;
  /** The instance picked in the form, for instance steps. */
  instance: string | null;
  /** The args entered so far (see `draftArgs`). */
  args: Record<string, unknown>;
}

export function fieldType(field: FieldSchema): string {
  if (field.enum) return field.enum.map((value) => JSON.stringify(value)).join(" | ");
  if (field.format) return field.format;
  if (field.type === "array") return `${field.items ? fieldType(field.items) : "unknown"}[]`;
  return field.type;
}

/** An example value for a field, for the copied command. */
export function exampleValue(field: FieldSchema): unknown {
  if (field.enum?.length) return field.enum[0];
  switch (field.format) {
    case "pubkey":
      return "<pubkey>";
    case "u8":
    case "u16":
    case "u32":
    case "u64":
    case "u128":
    case "i64":
    case "unixTimestamp":
      return "0";
    case "uri":
      return "https://example.com";
  }
  switch (field.type) {
    case "boolean":
      return false;
    case "integer":
      return 0;
    case "array":
      return [];
    case "object":
      return {};
    default:
      return "";
  }
}

function shellQuote(text: string): string {
  return `'${text.replace(/'/g, `'\\''`)}'`;
}

/**
 * The `a4 dev step` command that runs this step with `args` (example values
 * when there are none). A null `wallet` (a signer `a4 dev step` can't use)
 * leaves a placeholder.
 */
export function stepCommand(
  name: string,
  step: Step,
  args?: Record<string, unknown> | null,
  instanceId?: string | null,
  wallet: string | null = "default",
): string {
  const values =
    args ??
    Object.fromEntries(Object.entries(step.args.properties).map(([key, field]) => [key, exampleValue(field)]));
  const parts = ["a4 dev step", name];
  if (step.scope !== "create") parts.push("--instance", instanceId ?? "<instance id>");
  if (Object.keys(values).length > 0) parts.push("--args", shellQuote(JSON.stringify(values)));
  parts.push("--wallet", wallet ?? "<dev wallet>");
  return parts.join(" ");
}

function slotLabel(slot: AnatomySlot): string {
  switch (slot.kind) {
    case "computeBudget":
      return "compute budget";
    case "createAta":
      return `create ATA (owner ${slot.owner}, mint ${slot.mint})`;
    case "wrapSol":
      return `wrap SOL (${slot.amount})`;
    case "unwrapSol":
      return "unwrap SOL";
    case "fee":
      return `${slot.basis} fee (${slot.placement})`;
    case "template":
      return `${slot.program}.${slot.instruction}`;
    case "handler":
      return `handler ${slot.export} (${slot.programs.join(", ")})`;
    case "memo":
      return "memo (execution id)";
  }
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="panel-section">
      <h3>{title}</h3>
      {children}
    </section>
  );
}

function shortSig(signature: string): string {
  return signature.length > 16 ? `${signature.slice(0, 8)}…${signature.slice(-8)}` : signature;
}

function ExecutionRow({ execution, walletNames }: { execution: Execution; walletNames?: Record<string, string> }) {
  const [open, setOpen] = useState(false);
  const ok = execution.state === "completed";
  const failed = execution.updates.some((update) => update.type === "step.failed");
  const logs = execution.simulation.flatMap((simulation) => simulation.logs ?? []);
  return (
    <li className={`execution ${ok ? "ok" : failed ? "fail" : "pending"}`} data-testid="execution">
      <button className="execution-head" onClick={() => setOpen(!open)}>
        <span className="execution-state">{execution.rejected ? `rejected · ${execution.rejected}` : execution.state}</span>
        <span className="execution-who">
          {execution.devWallet ??
            (execution.wallet ? (walletNames?.[execution.wallet] ?? shortSig(execution.wallet)) : "")}
        </span>
        <span className="execution-at">{new Date(execution.lastAt).toLocaleTimeString()}</span>
      </button>
      {open ? (
        <div className="execution-body">
          {execution.executionId ? <div className="mono">execution {execution.executionId}</div> : null}
          {execution.instanceId ? <div className="mono">instance {execution.instanceId}</div> : null}
          {execution.transactions.map((transaction) => (
            <div key={transaction.index} className="mono">
              tx {transaction.index}: {transaction.state}
              {transaction.signature ? ` · ${transaction.signature}` : ""}
              {transaction.error ? ` · ${transaction.error}` : ""}
            </div>
          ))}
          {execution.error ? <div className="error-text">error: {JSON.stringify(execution.error)}</div> : null}
          {execution.args ? <pre className="code">{JSON.stringify(execution.args, null, 2)}</pre> : null}
          {logs.length > 0 ? (
            <details>
              <summary>simulation logs ({logs.length})</summary>
              <pre className="code logs">{logs.join("\n")}</pre>
            </details>
          ) : null}
        </div>
      ) : null}
    </li>
  );
}

export function StepPanel({ name, step, slots, executions, instanceId, walletNames, draft, onClose, children }: StepPanelProps) {
  const [copied, setCopied] = useState(false);
  const form = draft?.step === name ? draft : null;
  const command = form
    ? stepCommand(name, step, form.args, form.instance, form.wallet)
    : stepCommand(name, step, null, instanceId);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(command);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      setCopied(false);
    }
  };
  const fields = Object.entries(step.args.properties);
  return (
    <aside className="panel step-panel" data-testid="step-panel">
      <header className="panel-header">
        <div>
          <div className="panel-kicker">{step.scope} step</div>
          <h2>{name}</h2>
          {step.title ? <div className="panel-sub">{step.title}</div> : null}
        </div>
        <button className="icon" onClick={onClose} aria-label="Close">
          ×
        </button>
      </header>
      {step.description ? <p className="muted">{step.description}</p> : null}

      <Section title="Authority">
        <div>
          {authorityLabel(step.authority)}
          {step.access ? <span className="tag">{step.access} key</span> : null}
        </div>
        <div className="muted small">signers: {signerClasses(step).join(", ")}</div>
      </Section>

      <Section title={`Params (${fields.length})`}>
        {fields.length === 0 ? (
          <div className="muted">none</div>
        ) : (
          <table className="kv">
            <tbody>
              {fields.map(([key, field]) => (
                <tr key={key}>
                  <td className="mono">
                    {key}
                    {step.args.required.includes(key) ? "" : "?"}
                  </td>
                  <td className="mono muted">{fieldType(field)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Section>

      <Section title={`Guards (${step.guards.length})`}>
        {step.guards.length === 0 ? (
          <div className="muted">none</div>
        ) : (
          <ul className="guards">
            {step.guards.map((guard) => (
              <li key={guard.code}>
                <span className="guard-code">{guard.code}</span>
                <code>{guard.expr}</code>
                {guard.message ? <div className="muted small">{guard.message}</div> : null}
              </li>
            ))}
          </ul>
        )}
      </Section>

      <Section title="Build">
        {step.build.kind === "template" ? (
          step.build.instructions.map((instruction, index) => (
            <div key={index} className="instruction">
              <div className="mono">
                {instruction.program}.{instruction.instruction}
              </div>
              <table className="kv">
                <tbody>
                  {Object.entries(instruction.accounts).map(([account, expression]) => (
                    <tr key={account}>
                      <td className="mono">{account}</td>
                      <td className="mono muted">{expression}</td>
                    </tr>
                  ))}
                  {Object.entries(instruction.args).map(([arg, expression]) => (
                    <tr key={`arg:${arg}`}>
                      <td className="mono">arg {arg}</td>
                      <td className="mono muted">{expression}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ))
        ) : (
          <div>
            handler <span className="mono">{step.build.export}</span> over{" "}
            <span className="mono">{step.build.programs.join(", ")}</span>
          </div>
        )}
      </Section>

      {slots ? (
        <Section title="Transaction anatomy">
          <ol className="anatomy">
            {slots.map((slot, index) => (
              <li key={index} className={`slot ${slot.role}`}>
                <span className="slot-role">{slot.role}</span>
                {slotLabel(slot)}
              </li>
            ))}
          </ol>
        </Section>
      ) : null}

      {children}

      <Section title="Command">
        <pre className="code">{command}</pre>
        <button onClick={copy}>{copied ? "Copied" : "Copy as a4 dev step"}</button>
      </Section>

      <Section title={`Recent executions (${executions.length})`}>
        {executions.length === 0 ? (
          <div className="muted">none yet</div>
        ) : (
          <ul className="executions">
            {executions.map((execution) => (
              <ExecutionRow key={execution.key} execution={execution} walletNames={walletNames} />
            ))}
          </ul>
        )}
      </Section>
    </aside>
  );
}
