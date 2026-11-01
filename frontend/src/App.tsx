import React, { useState, useEffect, useMemo } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { registry } from './core/registry';
import { dashboardModule } from './modules/dashboard';
import { coreModule } from './modules/core';
import { todoModule } from './modules/todo';
import { reminderModule } from './modules/reminder';
import { financeModule } from './modules/finance';
import { portfolioModule } from './modules/portfolio';
import { recurringBillsModule } from './modules/recurring_bills';
import { webhooksModule } from './modules/webhooks';
import { aiModule } from './modules/ai';
import { footprintsModule } from './modules/footprints';
import { habitsModule } from './modules/habits';
import { lifeModule } from './modules/life';
import { moodModule } from './modules/mood';
import { timeBlocksModule } from './modules/time_blocks';
import { bookmarksModule } from './modules/bookmarks';
import { knowledgeModule } from './modules/knowledge';
import { agentModule } from './modules/agent';
import { adminModule } from './modules/admin';
import { useSystemEvents } from './core/useSystemEvents';
import { AppShell } from './components/AppShell';
import { AuthModal } from './components/AuthModal';
import { SettingsModal } from './components/SettingsModal';
import { ToastContainer } from './components/ToastContainer';
import { CommandPalette } from './components/CommandPalette';
import { useCommandPalette } from './components/CommandPalette/useCommandPalette';
import type { PaletteContext } from './components/CommandPalette/paletteRegistry';
import { AUTH_EXPIRED_EVENT } from './core/apiClient';
import { useSession, useRefreshSession } from './core/session';
import { Theme, getStoredTheme, setStoredTheme, applyTheme } from './core/theme';

const LinkPage = React.lazy(() =>
  import('./modules/link/LinkPage').then((m) => ({ default: m.LinkPage }))
);
const PublicTransparencyView = React.lazy(() =>
  import('./modules/finance/components/PublicTransparencyView').then((m) => ({
    default: m.PublicTransparencyView,
  }))
);

// Safely register modules only once without HMR re-registration warnings
[dashboardModule, coreModule, knowledgeModule, todoModule, reminderModule, bookmarksModule, financeModule, portfolioModule, recurringBillsModule, footprintsModule, habitsModule, moodModule, timeBlocksModule, lifeModule, webhooksModule, aiModule, agentModule, adminModule].forEach((mod) => {
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

/** The page a bot's `/link` message opens; it works signed out and owns its own sign-in prompt. */
const isLinkRoute = () => typeof window !== 'undefined' && window.location.pathname.replace(/\/+$/, '') === '/link';

const AppContent: React.FC = () => {
  const [onLinkRoute] = useState(isLinkRoute);
  const [activeModuleId, setActiveModuleId] = useState<string>('dashboard');
  // Set alongside activeModuleId only by a Quick Action jump, so the target
  // module's primary input autofocuses; any ordinary nav click clears it.
  const [focusModuleId, setFocusModuleId] = useState<string | null>(null);
  const [isAuthOpen, setIsAuthOpen] = useState(false);
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [theme, setTheme] = useState<Theme>(getStoredTheme());
  const [enrollToken, setEnrollToken] = useState<string | null>(() => {
    if (typeof window === 'undefined' || isLinkRoute()) return null;
    const params = new URLSearchParams(window.location.search);
    return params.get('enroll') || params.get('token');
  });
  const [publicFinanceToken] = useState<string | null>(() => {
    if (typeof window === 'undefined') return null;
    const params = new URLSearchParams(window.location.search);
    return params.get('public_finance');
  });

  const { user, isSignedIn, isLoading, isError } = useSession();
  const refreshSession = useRefreshSession();

  const { events, isConnected } = useSystemEvents(isSignedIn && !onLinkRoute);
  const palette = useCommandPalette(isSignedIn && !onLinkRoute && !publicFinanceToken);

  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  useEffect(() => {
    if (enrollToken) {
      setIsAuthOpen(true);
    }
  }, [enrollToken]);

  // Support decoupled module navigation events
  useEffect(() => {
    const onNavigate = (e: Event) => {
      const detail = (e as CustomEvent).detail;
      if (detail?.moduleId) {
        if (detail.moduleId === 'admin' && user && user.role !== 'admin') {
          return;
        }
        setActiveModuleId(detail.moduleId);
      }
    };
    const onAiPrompt = () => {
      setActiveModuleId('ai');
    };
    window.addEventListener('app:navigate', onNavigate);
    window.addEventListener('app:ai-prompt', onAiPrompt);
    return () => {
      window.removeEventListener('app:navigate', onNavigate);
      window.removeEventListener('app:ai-prompt', onAiPrompt);
    };
  }, [user]);

  useEffect(() => {
    if (activeModuleId === 'admin' && user && user.role !== 'admin') {
      setActiveModuleId('dashboard');
    }
  }, [activeModuleId, user]);

  // Signed out is a state, not an event: derive the prompt from the session
  // itself so it appears on a cold load as well as on an expiry, and so it
  // closes on its own once a session exists. Do not open during network errors / server restarts.
  useEffect(() => {
    if (!isLoading && !isError && !publicFinanceToken && !onLinkRoute) setIsAuthOpen(!isSignedIn);
  }, [isLoading, isSignedIn, isError, publicFinanceToken, onLinkRoute]);

  // The server rejected the session cookie on some *other* call. Re-read the
  // session so the rest of the UI stops showing data it can no longer refresh;
  // the effect above turns that into a prompt.
  useEffect(() => {
    const onExpired = () => {
      refreshSession();
    };
    window.addEventListener(AUTH_EXPIRED_EVENT, onExpired);
    return () => window.removeEventListener(AUTH_EXPIRED_EVENT, onExpired);
  }, [refreshSession]);

  const handleThemeChange = (newTheme: Theme) => {
    setTheme(newTheme);
    setStoredTheme(newTheme);
  };

  const handleSelectModule = (id: string) => {
    setActiveModuleId(id);
    setFocusModuleId(null);
  };

  const handleQuickNavigate = (id: string) => {
    setActiveModuleId(id);
    setFocusModuleId(id);
  };

  const paletteContext = useMemo<PaletteContext>(
    () => ({
      isAdmin: user?.role === 'admin',
      isDark: theme === 'dark',
      onSelectModule: handleSelectModule,
      onQuickNavigate: handleQuickNavigate,
      onToggleTheme: () => handleThemeChange(theme === 'dark' ? 'light' : 'dark'),
    }),
    // The handlers only call state setters, so they are stable in effect.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [user?.role, theme]
  );

  if (onLinkRoute) {
    return (
      <React.Suspense fallback={null}>
        <LinkPage />
      </React.Suspense>
    );
  }

  if (publicFinanceToken) {
    return (
      <React.Suspense fallback={null}>
        <div className="min-h-screen bg-[var(--color-canvas)] text-[var(--color-text-primary)] font-sans px-6 md:px-12 py-8 max-w-5xl mx-auto space-y-6">
          <header className="flex items-center justify-between pb-4 border-b border-neutral-200 dark:border-neutral-800">
            <div className="flex items-center gap-2">
              <span className="text-sm font-semibold tracking-tight text-neutral-900 dark:text-neutral-100 font-sans">
                Matrix Bot
              </span>
              <span className="text-xs text-neutral-400 font-mono">/</span>
              <span className="text-xs font-mono text-neutral-500">Public Fiscal Transparency</span>
            </div>
            <a
              href="/"
              className="text-xs font-medium text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-100 transition"
            >
              Go to App →
            </a>
          </header>
          <PublicTransparencyView publicToken={publicFinanceToken} />
        </div>
      </React.Suspense>
    );
  }

  return (
    <>
      <AppShell
        activeModuleId={activeModuleId}
        onSelectModule={handleSelectModule}
        onQuickNavigate={handleQuickNavigate}
        focusModuleId={focusModuleId}
        events={events}
        isConnected={isConnected}
        onOpenAuth={() => setIsAuthOpen(true)}
        onOpenSettings={() => setIsSettingsOpen(true)}
        onOpenPalette={palette.open}
        user={user}
        currentTheme={theme}
        onThemeChange={handleThemeChange}
      />

      <AuthModal
        isOpen={isAuthOpen}
        onClose={() => {
          setIsAuthOpen(false);
          setEnrollToken(null);
        }}
        onSuccess={() => {
          setEnrollToken(null);
          refreshSession();
        }}
        user={user}
        enrollToken={enrollToken}
      />

      <SettingsModal isOpen={isSettingsOpen} onClose={() => setIsSettingsOpen(false)} />
      <CommandPalette
        isOpen={palette.isOpen}
        onClose={palette.close}
        context={paletteContext}
      />
      <ToastContainer />
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
