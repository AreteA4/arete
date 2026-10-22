/**
 * Automations as a definition: the handlers and view sources, apart from the
 * client, signer, store and secrets that run them. The same definition runs
 * under `a4 dev` (against the project's dev environment) and in production
 * (`automations.use(definition)`), so moving from one to the other changes
 * configuration only.
 */

import { Arete } from '@usearete/sdk';
import type { ConnectedArete, ProtocolDefinition, ProtocolTypeMap, StackDefinition } from '@usearete/sdk';

import type { PolicyInput } from '../policy/policy';
import type { ViewSource } from '../sources/view';
import type { Automations, SourceHandle } from './automations';

/** Connects a stack's views: the dev environment's under `a4 dev`, the published stack in production. */
export type StackConnector = <T extends StackDefinition>(stack: T) => Promise<ConnectedArete<T>>;

/** What a definition's setup registers handlers and sources on. */
export interface AutomationsSetup<TTypes extends ProtocolTypeMap = ProtocolTypeMap> {
  readonly on: Automations<TTypes>['on'];
  source(source: ViewSource, options?: { exclusive?: boolean }): SourceHandle;
  /** Connect a stack for its views, to build view sources from. */
  readonly connect: StackConnector;
}

/** The policy fields a definition may set: the deployment and cluster come from where it runs. */
export type DefinitionPolicy = Omit<PolicyInput, 'deployments' | 'clusters'>;

export interface AutomationsDefinition<TTypes extends ProtocolTypeMap = ProtocolTypeMap> {
  readonly kind: 'automations';
  /** The protocol's generated definition, for its client. */
  readonly protocol: ProtocolDefinition<TTypes, any, any>;
  /**
   * The programs and caps the steps need. `a4 dev` signs under it; in
   * production, spread it into `definePolicy` with the deployment and
   * clusters. Without it `a4 dev` allows 1 SOL per transaction and no token.
   */
  readonly policy?: DefinitionPolicy;
  readonly setup: (automations: AutomationsSetup<TTypes>) => void | Promise<void>;
}

/**
 * Define automations for a protocol: handlers for its events and dispatched
 * triggers, and view sources. Export the definition as the module's default
 * for `a4 dev` (`[dev.automations] entry` in arete.toml).
 */
export function defineAutomations<TTypes extends ProtocolTypeMap>(
  protocol: ProtocolDefinition<TTypes, any, any>,
  setup: (automations: AutomationsSetup<TTypes>) => void | Promise<void>,
  options: { policy?: DefinitionPolicy } = {},
): AutomationsDefinition<TTypes> {
  return { kind: 'automations', protocol, setup, ...(options.policy ? { policy: options.policy } : {}) };
}

/** Whether `value` is a definition, from this or another copy of the package. */
export function isAutomationsDefinition(value: unknown): value is AutomationsDefinition {
  return (
    typeof value === 'object' &&
    value !== null &&
    (value as { kind?: unknown }).kind === 'automations' &&
    typeof (value as { setup?: unknown }).setup === 'function' &&
    typeof (value as { protocol?: unknown }).protocol === 'object'
  );
}

/** Connects with the SDK's defaults: `ARETE_API_KEY` or the `a4` login. */
export const connectStack: StackConnector = (stack) => Arete.connect(stack) as never;
