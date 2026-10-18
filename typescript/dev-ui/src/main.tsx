import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import "@xyflow/react/dist/style.css";
import "./styles.css";

import { App } from "./App";
import type { StepDraft } from "./components/StepPanel";
import { activeInstances } from "./model";
import { DevTools } from "./run/DevTools";
import { RunStepForm } from "./run/RunStepForm";
import { BrowserWalletProvider, useBrowserWallet, WalletBar } from "./wallet/BrowserWallet";
import type { Model } from "./model";

function StepActions({
  step,
  instance,
  model,
  onDraft,
}: {
  step: string;
  instance: string | null;
  model: Model;
  onDraft(draft: StepDraft): void;
}) {
  const browser = useBrowserWallet();
  const spec = model.snapshot?.spec?.steps[step];
  if (!spec) return null;
  const connected = browser?.wallet && browser.account ? browser : null;
  return (
    <RunStepForm
      name={step}
      step={spec}
      instances={activeInstances(model)}
      selectedInstance={instance}
      onDraft={onDraft}
      extraSigners={
        connected
          ? [{ id: "browser", label: `${connected.wallet!.name} ${connected.account!.address.slice(0, 4)}…${connected.account!.address.slice(-4)} (browser wallet)` }]
          : []
      }
      runWithExtra={connected ? (_id, input, onPreview) => connected.run(input, onPreview) : undefined}
    />
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <BrowserWalletProvider>
      <App
        renderTools={() => (
          <DevTools>
            <WalletBar />
          </DevTools>
        )}
        renderStepActions={(step, instance, model, onDraft) => (
          <StepActions step={step} instance={instance} model={model} onDraft={onDraft} />
        )}
      />
    </BrowserWalletProvider>
  </StrictMode>,
);
