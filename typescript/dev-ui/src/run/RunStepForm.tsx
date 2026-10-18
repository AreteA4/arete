import { useEffect, useMemo, useState } from "react";

import { fetchInstance } from "../api";
import type { StepDraft } from "../components/StepPanel";
import type { TrackedInstance } from "../model";
import type { Step } from "../types";
import { buildArgs, draftArgs, fieldInputs, initialValues, suggestedWallet, type FieldInput } from "./args";
import {
  isPrepared,
  prepareStep,
  runStep,
  submitStep,
  type PreparedPreview,
  type StepResult,
} from "./runApi";
import { TransactionPreview } from "./TransactionPreview";
import { sol, useWallets } from "./useWallets";

export interface RunStepFormProps {
  name: string;
  step: Step;
  instances: TrackedInstance[];
  selectedInstance: string | null;
  /** Extra signers (a connected browser wallet), rendered after the dev wallets. */
  extraSigners?: Array<{ id: string; label: string }>;
  /** Prepare, sign and submit with an extra signer. */
  /** Called with the form's choices as they change (the copied command follows them). */
  onDraft?(draft: StepDraft): void;
  runWithExtra?(id: string, input: { step: string; instance: string | null; args: Record<string, unknown> }, onPreview: (preview: PreparedPreview) => void): Promise<StepResult>;
}

function FieldControl({
  input,
  value,
  error,
  onChange,
  pubkeys,
}: {
  input: FieldInput;
  value: string;
  error?: string;
  onChange(value: string): void;
  pubkeys: Array<{ name: string; pubkey: string }>;
}) {
  const id = `arg-${input.name}`;
  let control;
  if (input.kind === "enum") {
    control = (
      <select id={id} value={value} onChange={(event) => onChange(event.target.value)}>
        {input.field.enum?.map((option) => (
          <option key={option}>{option}</option>
        ))}
      </select>
    );
  } else if (input.kind === "boolean") {
    control = <input id={id} type="checkbox" checked={value === "true"} onChange={(event) => onChange(String(event.target.checked))} />;
  } else if (input.kind === "json") {
    control = <textarea id={id} rows={2} value={value} placeholder={input.placeholder} onChange={(event) => onChange(event.target.value)} />;
  } else {
    control = (
      <input
        id={id}
        value={value}
        placeholder={input.placeholder}
        inputMode={input.kind === "integer" || input.kind === "number" ? "numeric" : undefined}
        onChange={(event) => onChange(event.target.value)}
        list={input.kind === "pubkey" ? `${id}-wallets` : undefined}
      />
    );
  }
  return (
    <label className="field" htmlFor={id}>
      <span className="field-name mono">
        {input.name}
        {input.required ? "" : "?"} <span className="muted">{input.field.format ?? input.field.type}</span>
      </span>
      {control}
      {input.kind === "pubkey" ? (
        <>
          <datalist id={`${id}-wallets`}>
            {pubkeys.map((wallet) => (
              <option key={wallet.pubkey} value={wallet.pubkey}>
                {wallet.name}
              </option>
            ))}
          </datalist>
          <span className="pick">
            {pubkeys.map((wallet) => (
              <button key={wallet.pubkey} type="button" className="link small" onClick={() => onChange(wallet.pubkey)}>
                {wallet.name}
              </button>
            ))}
          </span>
        </>
      ) : null}
      {error ? <span className="error-text small">{error}</span> : null}
    </label>
  );
}

/** A rejection's reason, from its detail (the runtime's message). */
export function rejectionText(result: StepResult): string | null {
  const detail = result.detail as { message?: unknown; error?: unknown; detail?: unknown } | undefined;
  if (!result.rejected || !detail) return null;
  const nested = detail.detail as { message?: unknown } | string | undefined;
  const text = detail.message ?? detail.error ?? (typeof nested === "string" ? nested : nested?.message);
  return typeof text === "string" ? text : null;
}

/** What to do about a rejection (terminal states carry one). */
export function rejectionHint(result: StepResult): string | null {
  const hint = (result.detail as { hint?: unknown } | undefined)?.hint;
  return result.rejected && typeof hint === "string" ? hint : null;
}

export function RunStepForm({ name, step, instances, selectedInstance, extraSigners = [], onDraft, runWithExtra }: RunStepFormProps) {
  const { wallets } = useWallets();
  const [values, setValues] = useState(() => initialValues(step.args));
  const [instance, setInstance] = useState<string>(selectedInstance ?? "");
  const [signer, setSigner] = useState<string>("");
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [preview, setPreview] = useState<PreparedPreview | null>(null);
  const [result, setResult] = useState<StepResult | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const needsInstance = step.scope !== "create";
  // Roles and accounts come from the instance itself (events don't carry them).
  const [detail, setDetail] = useState<{ roles?: Record<string, string>; accounts?: Record<string, string> } | undefined>();
  useEffect(() => {
    setDetail(undefined);
    if (!needsInstance || !instance) return;
    let cancelled = false;
    fetchInstance(instance).then(
      (read) => {
        if (!cancelled) setDetail(read as { roles?: Record<string, string>; accounts?: Record<string, string> });
      },
      () => undefined,
    );
    return () => {
      cancelled = true;
    };
  }, [instance, needsInstance]);
  const current = instances.find((candidate) => candidate.id === instance);
  const suggested = suggestedWallet(step.authority, detail ?? current, wallets);

  useEffect(() => {
    setValues(initialValues(step.args));
    setPreview(null);
    setResult(null);
    setFailure(null);
  }, [name, step]);
  useEffect(() => {
    if (selectedInstance) setInstance(selectedInstance);
  }, [selectedInstance]);
  // The role holder's dev wallet when the instance names one; else keep the
  // choice (a wallet refresh must not undo it); else the first dev wallet.
  // A browser wallet the user picked stays picked when the instance (and so
  // the suggestion) changes.
  useEffect(() => {
    if (suggested) setSigner((current) => (current === "" || current.startsWith("dev:") ? `dev:${suggested}` : current));
  }, [suggested]);
  // A signer that went away (a disconnected browser wallet) falls back to
  // the suggested or first dev wallet, so Run never silently does nothing.
  const extraIds = extraSigners.map((extra) => extra.id).join("\n");
  useEffect(() => {
    const known = signer.startsWith("dev:") || extraIds.split("\n").includes(signer);
    if (signer && known) return;
    const fallback = suggested ?? wallets[0]?.name;
    setSigner(fallback ? `dev:${fallback}` : "");
  }, [wallets, signer, suggested, extraIds]);

  const inputs = useMemo(() => fieldInputs(step.args), [step]);
  const names = useMemo(() => Object.fromEntries(wallets.map((wallet) => [wallet.pubkey, wallet.name])), [wallets]);

  const collect = () => {
    const built = buildArgs(step.args, values);
    setErrors(built.errors);
    if (Object.keys(built.errors).length > 0) return null;
    if (needsInstance && !instance) {
      setFailure("Pick an instance");
      return null;
    }
    return { step: name, instance: needsInstance ? instance : null, args: built.args };
  };

  const guard = async (label: string, action: () => Promise<void>) => {
    setBusy(label);
    setFailure(null);
    setResult(null);
    try {
      await action();
    } catch (error) {
      setFailure((error as Error).message);
    } finally {
      setBusy(null);
    }
  };

  const devWallet = signer.startsWith("dev:") ? signer.slice(4) : null;

  // A preview is of the exact args, instance and signer it was prepared
  // with; signing it after any of them changed would send the old ones.
  useEffect(() => {
    setPreview(null);
  }, [values, instance, signer]);

  useEffect(() => {
    onDraft?.({
      step: name,
      wallet: devWallet,
      instance: needsInstance && instance ? instance : null,
      args: draftArgs(step.args, values),
    });
  }, [onDraft, name, step, devWallet, needsInstance, instance, values]);

  const onPreview = () =>
    guard("preview", async () => {
      const input = collect();
      if (!input || !devWallet) return;
      setResult(null);
      const prepared = await prepareStep({ ...input, wallet: devWallet });
      if (isPrepared(prepared)) setPreview(prepared);
      else {
        setPreview(null);
        setResult(prepared);
      }
    });

  const onSubmit = () =>
    guard("submit", async () => {
      if (!preview) return;
      setResult(await submitStep(preview.executionId));
      setPreview(null);
    });

  const onRun = () =>
    guard("run", async () => {
      const input = collect();
      if (!input) return;
      setPreview(null);
      if (devWallet) setResult(await runStep({ ...input, wallet: devWallet }));
      else if (runWithExtra && extraSigners.some((extra) => extra.id === signer)) setResult(await runWithExtra(signer, input, setPreview));
      else throw new Error("Pick who signs: that wallet is no longer connected");
    });

  return (
    <section className="panel-section run" data-testid="run-form">
      <h3>Run</h3>
      <label className="field">
        <span className="field-name">Run as</span>
        <select value={signer} onChange={(event) => setSigner(event.target.value)} data-testid="run-as">
          {wallets.map((wallet) => (
            <option key={wallet.name} value={`dev:${wallet.name}`}>
              {wallet.name}
              {wallet.name === suggested ? ` (${step.authority.kind === "role" ? `role ${step.authority.role}` : step.authority.kind})` : ""}
              {" · "}
              {sol(wallet.lamports)}
            </option>
          ))}
          {extraSigners.map((extra) => (
            <option key={extra.id} value={extra.id}>
              {extra.label}
            </option>
          ))}
        </select>
      </label>
      {needsInstance ? (
        <label className="field">
          <span className="field-name">Instance</span>
          <select value={instance} onChange={(event) => setInstance(event.target.value)} data-testid="run-instance">
            <option value="">— pick —</option>
            {instances.map((candidate) => (
              <option key={candidate.id} value={candidate.id}>
                {candidate.id.slice(0, 8)} · {candidate.state}
              </option>
            ))}
          </select>
        </label>
      ) : null}
      {inputs.map((input) => (
        <FieldControl
          key={input.name}
          input={input}
          value={values[input.name] ?? ""}
          error={errors[input.name]}
          pubkeys={wallets}
          onChange={(value) => setValues({ ...values, [input.name]: value })}
        />
      ))}
      <div className="buttons">
        {devWallet ? (
          <button onClick={onPreview} disabled={busy !== null} data-testid="preview-button">
            {busy === "preview" ? "Preparing…" : "Preview"}
          </button>
        ) : null}
        <button className="primary" onClick={onRun} disabled={busy !== null} data-testid="run-button">
          {busy === "run" ? "Running…" : devWallet ? "Run" : "Prepare, sign in wallet, submit"}
        </button>
      </div>
      {preview ? (
        <>
          <TransactionPreview preview={preview} names={names} />
          {devWallet ? (
            <button className="primary" onClick={onSubmit} disabled={busy !== null} data-testid="submit-button">
              {busy === "submit" ? "Submitting…" : `Sign with ${devWallet} and submit`}
            </button>
          ) : null}
        </>
      ) : null}
      {failure ? (
        <div className="error-text" data-testid="run-error">
          {failure}
        </div>
      ) : null}
      {result ? (
        <div className={`result ${result.state === "completed" ? "ok" : "fail"}`} data-testid="run-result">
          <strong>{result.state}</strong>
          {result.rejected ? ` · ${result.rejected}` : ""}
          {rejectionText(result) ? <div className="small" data-testid="run-rejection">{rejectionText(result)}</div> : null}
          {rejectionHint(result) ? (
            <div className="small muted" data-testid="run-hint">
              {rejectionHint(result)}
            </div>
          ) : null}
          {result.instanceId ? <div className="mono small">instance {result.instanceId}</div> : null}
          {(result.transactions ?? []).map((transaction) => (
            <div key={transaction.index} className="mono small">
              {transaction.state} {transaction.signature ?? ""} {transaction.error ?? ""}
            </div>
          ))}
        </div>
      ) : null}
    </section>
  );
}
