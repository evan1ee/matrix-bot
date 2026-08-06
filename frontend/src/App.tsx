import React, { useState, useEffect } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { registry } from './core/registry';
import { coreModule } from './modules/core';
import { todoModule } from './modules/todo';
import { reminderModule } from './modules/reminder';
import { useSystemEvents } from './core/useSystemEvents';
import { VercelShell } from './components/VercelShell';
import { AuthModal } from './components/AuthModal';
import { getAuthToken } from './core/apiClient';
import { Theme, getStoredTheme, setStoredTheme, applyTheme } from './core/theme';

// Register all modules into the Plugin System Registry
registry.register(coreModule);
registry.register(todoModule);
registry.register(reminderModule);

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
  const [token, setToken] = useState<string | null>(getAuthToken());
  const [theme, setTheme] = useState<Theme>(getStoredTheme());

  const { events, isConnected } = useSystemEvents();

  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  const handleThemeChange = (newTheme: Theme) => {
    setTheme(newTheme);
    setStoredTheme(newTheme);
  };

  return (
    <>
      <VercelShell
        activeModuleId={activeModuleId}
        onSelectModule={setActiveModuleId}
        events={events}
        isConnected={isConnected}
        onOpenAuth={() => setIsAuthOpen(true)}
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
