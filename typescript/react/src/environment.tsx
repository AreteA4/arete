import React from 'react';
import {
  resolveEnvironmentSwitch,
  type ClientEnvironment,
  type StackDefinition,
} from '@usearete/sdk';

import { useOptionalAreteContext } from './provider';
import type { ClientLookupOptions } from './types';

/**
 * The world the enclosing `AreteProvider`'s clients talk to: `'dev'` for
 * the project's dev environment, `'production'` otherwise. With a stack,
 * the connected client's own `client.environment`; before it connects (or
 * without a stack), what the provider's `environment` decides. Outside a
 * provider, `'production'`.
 */
export function useAreteEnvironment(
  stack?: StackDefinition,
  options?: ClientLookupOptions
): ClientEnvironment {
  const context = useOptionalAreteContext();
  // Read again when a client connects.
  const [, setRevision] = React.useState(0);
  const subscribe = context?.subscribeToClientChanges;
  React.useEffect(
    () => subscribe?.(() => setRevision((revision) => revision + 1)),
    [subscribe]
  );
  const client = context?.getClient(stack ?? context.config.stack, options);
  if (client) return client.environment;
  if (!context) return 'production';
  try {
    return resolveEnvironmentSwitch(context.config.environment).mode;
  } catch {
    // A malformed switch fails the client with INVALID_CONFIG; the badge
    // only reports where a working client goes.
    return 'production';
  }
}

export interface AreteEnvironmentBadgeProps {
  /** Text shown in dev mode. Defaults to `Dev environment`. */
  label?: React.ReactNode;
  className?: string;
  style?: React.CSSProperties;
  /** Report this stack's client (see {@link useAreteEnvironment}). */
  stack?: StackDefinition;
}

/**
 * A small badge that says the app is talking to the dev environment.
 * Renders nothing in production, so it can stay in the app's status or
 * diagnostics area. Carries `data-arete-environment="dev"` for styling.
 */
export function AreteEnvironmentBadge({
  label = 'Dev environment',
  className,
  style,
  stack,
}: AreteEnvironmentBadgeProps): React.ReactElement | null {
  const environment = useAreteEnvironment(stack);
  if (environment !== 'dev') return null;
  return (
    <span
      role="status"
      data-arete-environment="dev"
      className={className}
      style={style}
    >
      {label}
    </span>
  );
}
