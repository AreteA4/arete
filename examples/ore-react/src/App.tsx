import { OreDashboard } from './components';
import { AreteProvider } from '@usearete/react';
import { ThemeProvider } from './hooks/useTheme';

// Use your own publishable key in production
const PUBLISHABLE_KEY = 'hspk_alt8MN3BmJebxARE3IlOnnaAEibCrqqXfdG5VoGW';

export default function App() {
  return (
    <ThemeProvider>
      <AreteProvider
        autoConnect={true}
        auth={{
          publishableKey: PUBLISHABLE_KEY,
        }}
      >
        <OreDashboard />
      </AreteProvider>
    </ThemeProvider>
  );
}
