import { OreDashboard } from './components';
import { AreteProvider } from '@usearete/react';
import { ThemeProvider } from './hooks/useTheme';

const PUBLISHABLE_KEY = import.meta.env.VITE_ARETE_PUBLISHABLE_KEY ?? 'hspk_alt8MN3BmJebxARE3IlOnnaAEibCrqqXfdG5VoGW';

export default function App() {
  return (
    <ThemeProvider>
      <AreteProvider
        autoConnect={true}
        auth={PUBLISHABLE_KEY ? { publishableKey: PUBLISHABLE_KEY } : undefined}
      >
        <OreDashboard />
      </AreteProvider>
    </ThemeProvider>
  );
}
