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
import type { Automations, ScheduleOptions, SourceHandle } from './automations';
import { checkScheduleEvery, type ScheduleEvery } from './time';

/** Connects a stack's views: the dev environment's under `a4 dev`, the published stack in production. */
export type StackConnector = <T extends StackDefinition>(stack: T) => Promise<ConnectedArete<T>>;

/** What a definition's setup registers handlers and sources on. */
export interface AutomationsSetup<TTypes extends ProtocolTypeMap = ProtocolTypeMap> {
  readonly on: Automations<TTypes>['on'];
  /**
   * Dispatch the trigger `name` once per slot of `every` (see
   * `Automations.schedule`); register its handler with `on(name, ...)`.
   */
  schedule(name: string, options: ScheduleOptions): void;
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

/** What a definition registers, without running it: for deploy configuration and tools. */
export interface DefinitionManifest {
  /** The protocol's name. */
  readonly protocol: string;
  /** Protocol events with a handler: the events a webhook endpoint subscribes to. */
  readonly events: readonly string[];
  /** Schedules, by name; a host's cron must tick at least as often as the shortest. */
  readonly schedules: readonly { readonly name: string; readonly every: ScheduleEvery }[];
  /** View sources: `list` sources can be polled from a cron; `append` sources need a long-running worker. */
  readonly sources: readonly { readonly name: string; readonly kind: ViewSource['kind'] }[];
  /** Handlers for triggers that are not events, schedules or sources: your own `dispatch` calls. */
  readonly triggers: readonly string[];
}

/**
 * A stand-in for a connected stack: any property path is another stand-in,
 * so a definition can build its view sources without a connection.
 */
function stackStandIn(): unknown {
  const target = () => undefined;
  const proxy: unknown = new Proxy(target, {
    get: (_target, key) => (key === 'then' ? undefined : proxy),
    apply: () => proxy,
  });
  return proxy;
}

/**
 * Run a definition's setup against a recorder and report what it registers.
 * Nothing connects: stacks are stand-ins, so a definition that reads a view
 * during setup (rather than in its sources and handlers) cannot be described.
 */
export async function describeDefinition(definition: AutomationsDefinition): Promise<DefinitionManifest> {
  const handlers: string[] = [];
  const schedules: { name: string; every: ScheduleEvery }[] = [];
  const sources: { name: string; kind: ViewSource['kind'] }[] = [];
  const setup: AutomationsSetup = {
    on: ((name: string) => {
      handlers.push(name);
      return setup;
    }) as never,
    schedule(name, options) {
      checkScheduleEvery(options.every);
      schedules.push({ name, every: options.every });
    },
    source(source) {
      sources.push({ name: source.name, kind: source.kind });
      return { stop: async () => {} };
    },
    connect: (async () => stackStandIn()) as never,
  };
  await definition.setup(setup);
  const declared = new Set(Object.keys((definition.protocol as { events?: object }).events ?? {}));
  const dispatched = new Set([...schedules, ...sources].map((entry) => entry.name));
  const unique = [...new Set(handlers)];
  return {
    protocol: definition.protocol.name,
    events: unique.filter((name) => declared.has(name)),
    schedules,
    sources,
    triggers: unique.filter((name) => !declared.has(name) && !dispatched.has(name)),
  };
}
