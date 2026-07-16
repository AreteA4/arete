interface ConnectionBadgeProps {
  isConnected: boolean;
  connectionState: string;
  error: Error | null;
}

export function ConnectionBadge({ isConnected, connectionState, error }: ConnectionBadgeProps) {
  const label = isConnected ? 'Connected' : error ? 'Offline' : connectionState === 'connecting' ? 'Connecting' : 'Disconnected';
  return (
    <a
      className="flex min-h-11 items-center gap-2 rounded-full bg-white px-3 text-xs font-medium text-stone-600 shadow-sm dark:bg-stone-800 dark:text-stone-300 dark:ring-1 dark:ring-stone-700"
      href="https://docs.arete.run"
      target="_blank"
      rel="noreferrer"
      title={error?.message}
    >
      <i className={`h-1.5 w-1.5 rounded-full ${isConnected ? 'bg-emerald-500' : 'bg-amber-500'}`} aria-hidden="true" />
      {label}
    </a>
  );
}
