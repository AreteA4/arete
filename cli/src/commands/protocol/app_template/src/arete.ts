/**
 * The protocol client the app uses: the one `a4 dev up` generates for the
 * dev environment, or the published one after `a4 install --locked`. Both
 * live at the same path, so the app code is the same in dev and production.
 */
import { createProtocolClient } from "@usearete/sdk";

import protocol from "../../generated/typescript/protocols/{{slug}}/{{slug}}.js";

export const client = createProtocolClient(protocol, {
  auth: { publishableKey: import.meta.env.VITE_ARETE_PUBLISHABLE_KEY },
});

/**
 * Whether the client targets a dev environment: dev-only features (the dev
 * panel, funding, dev wallets) render only then. This is the check
 * `clientCluster` in `@usearete/automations` makes. Temporary: once the SDK
 * exposes the environment on the client, gate on
 * `client.environment === "dev"` instead.
 */
export const isDev =
  (client.definition.deployment as { readonly environment?: string } | undefined)?.environment === "dev";
