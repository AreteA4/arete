interface ConnectionBadgeProps {
  isConnected: boolean;
}

export function ConnectionBadge({ isConnected }: ConnectionBadgeProps) {
  return (
    <a href="https://docs.arete.run" target="_blank" rel="noreferrer" className="text-stone-600 dark:text-stone-300">
      <div className="fixed bottom-6 right-6 flex items-center gap-2 px-3 py-1.5 bg-white dark:bg-stone-800 rounded-full shadow-sm dark:shadow-none dark:ring-1 dark:ring-stone-700">
        <div className={`w-1.5 h-1.5 rounded-full ${isConnected ? 'bg-emerald-500' : 'bg-amber-500'}`} />
        <span className={`text-xs font-medium ${isConnected ? 'text-stone-600 dark:text-stone-300' : 'text-amber-600 dark:text-amber-400'}`}>
          {isConnected ? 'Connected' : 'Connecting'}
        </span>
      </div>
    </a>
  );
}
