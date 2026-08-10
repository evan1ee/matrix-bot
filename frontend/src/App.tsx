import React, { useState, useEffect } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { registry } from './core/registry';
import { coreModule } from './modules/core';
import { todoModule } from './modules/todo';
import { reminderModule } from './modules/reminder';
import { financeModule } from './modules/finance';
import { portfolioModule } from './modules/portfolio';
import { recurringBillsModule } from './modules/recurring_bills';
import { useSystemEvents } from './core/useSystemEvents';
import { AppShell } from './components/AppShell';
import { AuthModal } from './components/AuthModal';
import { SettingsModal } from './components/SettingsModal';
import { AUTH_EXPIRED_EVENT, getAuthToken } from './core/apiClient';
import { Theme, getStoredTheme, setStoredTheme, applyTheme } from './core/theme';

// Safely register modules only once without HMR re-registration warnings
[coreModule, todoModule, reminderModule, financeModule, portfolioModule, recurringBillsModule].forEach((mod) => {
  if (!registry.get(mod.id)) {
    registry.register(mod);
  }
});

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 5000,
      retry: 1,
    },
  },
});

const AppContent: React.FC = () => {
  const [activeModuleId, setActiveModuleId] = useState<string>('core');
  const [isAuthOpen, setIsAuthOpen] = useState(false);
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [token, setToken] = useState<string | null>(getAuthToken());
  const [theme, setTheme] = useState<Theme>(getStoredTheme());

  const { events, isConnected } = useSystemEvents(token);

  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  // A rejected token has already been cleared from storage; drop it from state
  // and open the login prompt so the session can be re-established.
  useEffect(() => {
    const onExpired = () => {
      setToken(null);
      setIsAuthOpen(true);
    };
    window.addEventListener(AUTH_EXPIRED_EVENT, onExpired);
    return () => window.removeEventListener(AUTH_EXPIRED_EVENT, onExpired);
  }, []);

  const handleThemeChange = (newTheme: Theme) => {
    setTheme(newTheme);
    setStoredTheme(newTheme);
  };

  return (
    <>
      <AppShell
        activeModuleId={activeModuleId}
        onSelectModule={setActiveModuleId}
        events={events}
        isConnected={isConnected}
        onOpenAuth={() => setIsAuthOpen(true)}
        onOpenSettings={() => setIsSettingsOpen(true)}
        userToken={token}
        currentTheme={theme}
        onThemeChange={handleThemeChange}
      />

      <AuthModal
        isOpen={isAuthOpen}
        onClose={() => setIsAuthOpen(false)}
        onSuccess={(newToken) => setToken(newToken)}
        userToken={token}
      />

      <SettingsModal isOpen={isSettingsOpen} onClose={() => setIsSettingsOpen(false)} />
    </>
  );
};

export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <AppContent />
    </QueryClientProvider>
  );
}

export default App;
