/**
 * The protocol client the app uses. The same client runs in dev and in
 * production: `a4 dev run` sets the two public variables below, which send
 * it to the dev environment through the edge `a4 dev up` serves. Browsers
 * read no process variables, so the app passes them on itself. Through the
 * edge the page needs no Arete key; without them it is production.
 */
import { createProtocolClient } from "@usearete/sdk";

import protocol from "../../generated/typescript/protocols/{{slug}}/{{slug}}.js";

export const client = createProtocolClient(protocol, {
  auth: { publishableKey: import.meta.env.VITE_ARETE_PUBLISHABLE_KEY },
  environment:
    import.meta.env.VITE_ARETE_ENVIRONMENT === "dev"
      ? { kind: "dev", url: import.meta.env.VITE_ARETE_DEV_URL }
      : "production",
});

/**
 * Whether the client targets a dev environment: dev-only features (the dev
 * panel, funding, dev wallets) render only then.
 */
export const isDev = client.environment === "dev";
