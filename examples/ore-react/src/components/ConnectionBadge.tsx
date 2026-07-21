import { useArete } from '@usearete/react';
import { ORE_STREAM_STACK } from '../generated/ore-stack';

// `useArete` shares the provider's cached client, so any component can read
// connection status directly — no prop drilling from a top-level component.
export function ConnectionBadge() {
  const arete = useArete(ORE_STREAM_STACK);
  const issue = arete.socketIssue;
  const label = arete.status === 'connected'
    ? issue ? 'Connected with issue' : 'Connected'
    : arete.status === 'error'
      ? 'Offline'
      : arete.status === 'reconnecting'
        ? 'Reconnecting'
        : arete.status === 'connecting'
          ? 'Connecting'
          : 'Disconnected';
  // `socketIssue` carries non-fatal stream problems reported by the server
  // (e.g. subscription limits) while the connection itself stays up.
  return (
    <div
      className="flex min-h-11 items-center gap-2 rounded-full bg-white px-3 text-xs font-medium text-stone-600 shadow-sm dark:bg-stone-800 dark:text-stone-300 dark:ring-1 dark:ring-stone-700"
      title={issue?.message ?? arete.error?.message}
      aria-live="polite"
    >
      <i
        className={`h-1.5 w-1.5 rounded-full ${arete.isConnected && !issue ? 'bg-emerald-500' : 'bg-amber-500'}`}
        aria-hidden="true"
      />
      {label}
      {arete.canRetry && (
        /* retry rejects for imperative composition; arete.error owns failure
           presentation for this event handler. */
        <button
          type="button"
          className="underline decoration-stone-300 underline-offset-2 hover:text-stone-900 dark:decoration-stone-600 dark:hover:text-white"
          onClick={() => void arete.retry().catch(() => undefined)}
        >
          Retry
        </button>
      )}
    </div>
  );
}
