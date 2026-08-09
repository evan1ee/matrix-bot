import React, { useEffect, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { registry } from '../core/registry';
import { fetchApi } from '../core/apiClient';
import { SystemEvent } from '../core/types';
import { TodoListResponse } from '../modules/todo';
import { ReminderListResponse } from '../modules/reminder';
import { ThemeToggle } from './ThemeToggle';
import { Theme } from '../core/theme';
import { Shield, Radio, CheckSquare, Clock, LogIn, Bot, Settings } from 'lucide-react';

export interface UserProfileResponse {
  username: string;
  display_name?: string | null;
  avatar_url?: string | null;
}

interface AppShellProps {
  activeModuleId: string;
  onSelectModule: (id: string) => void;
  events: SystemEvent[];
  isConnected: boolean;
  onOpenAuth: () => void;
  onOpenSettings: () => void;
  userToken: string | null;
  currentTheme: Theme;
  onThemeChange: (theme: Theme) => void;
}

export const AppShell: React.FC<AppShellProps> = ({
  activeModuleId,
  onSelectModule,
  events,
  isConnected,
  onOpenAuth,
  onOpenSettings,
  userToken,
  currentTheme,
  onThemeChange,
}) => {
  const [avatarObjectUrl, setAvatarObjectUrl] = useState<string | null>(null);
  const modules = registry.getAll();
  const activeModule = registry.get(activeModuleId) || modules[0];

  const { data: todosData } = useQuery<TodoListResponse>({
    queryKey: ['todos'],
    queryFn: () => fetchApi<TodoListResponse>('/todos?limit=100'),
    enabled: !!userToken,
  });

  const { data: remindersData } = useQuery<ReminderListResponse>({
    queryKey: ['reminders'],
    queryFn: () => fetchApi<ReminderListResponse>('/reminders?limit=100'),
    enabled: !!userToken,
  });

  const { data: userProfile } = useQuery<UserProfileResponse>({
    queryKey: ['userProfile', userToken],
    queryFn: () => fetchApi<UserProfileResponse>('/auth/me'),
    enabled: !!userToken,
  });

  const totalTodos = todosData?.items.length || 0;
  const activeReminders = remindersData?.items.length || 0;

  const displayName = userProfile?.display_name || userProfile?.username;

  // The media proxy is authenticated, so the avatar can't be a bare <img src>.
  // Fetch it with the bearer token and hand <img> an object URL instead — this
  // keeps the token in a header rather than in a URL the browser would log.
  const avatarPath = userProfile?.avatar_url || null;

  useEffect(() => {
    if (!avatarPath || !userToken) {
      setAvatarObjectUrl(null);
      return;
    }

    let cancelled = false;
    let objectUrl: string | null = null;

    fetch(avatarPath, { headers: { Authorization: `Bearer ${userToken}` } })
      .then((res) => (res.ok ? res.blob() : Promise.reject(new Error('Avatar unavailable'))))
      .then((blob) => {
        if (cancelled) return;
        objectUrl = URL.createObjectURL(blob);
        setAvatarObjectUrl(objectUrl);
      })
      .catch(() => {
        if (!cancelled) setAvatarObjectUrl(null);
      });

    return () => {
      cancelled = true;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [avatarPath, userToken]);

  return (
    <div className="min-h-screen bg-white dark:bg-black text-neutral-900 dark:text-neutral-100 flex flex-col font-sans transition-colors duration-150">
      {/* Masthead Header */}
      <header className="border-b border-neutral-200 dark:border-neutral-800 px-6 py-4">
        <div className="max-w-6xl mx-auto flex items-center justify-between">
          <div className="flex items-center gap-2.5">
            <div className="w-7 h-7 rounded bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black flex items-center justify-center font-bold">
              <Bot className="w-4 h-4" />
            </div>
            <span className="text-sm font-semibold tracking-tight text-neutral-900 dark:text-neutral-100">
              Matrix Halo Bot
            </span>
          </div>

          <div className="flex items-center gap-3">
            <div className="flex items-center gap-2 px-2.5 py-1 rounded-full bg-neutral-100 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 text-xs font-mono">
              <span
                className={`w-2 h-2 rounded-full ${
                  !userToken
                    ? 'bg-neutral-400 dark:bg-neutral-600'
                    : isConnected
                    ? 'bg-emerald-500 dark:bg-emerald-400 animate-pulse'
                    : 'bg-amber-500 dark:bg-amber-400'
                }`}
              />
              <span className="text-neutral-700 dark:text-neutral-300">
                {!userToken ? 'Auth Required' : isConnected ? 'SSE Live' : 'Reconnecting...'}
              </span>
            </div>

            {/* Matrix User Profile / Login Button */}
            <button
              onClick={onOpenAuth}
              className="inline-flex items-center gap-2 px-3 py-1.5 rounded-full bg-neutral-100 dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-xs font-medium text-neutral-800 dark:text-neutral-200 hover:bg-neutral-200 dark:hover:bg-neutral-850 transition cursor-pointer"
              title={userToken ? `Logged in as ${displayName || 'User'}` : 'Login / Token'}
            >
              {userToken ? (
                avatarObjectUrl ? (
                  <>
                    <img
                      src={avatarObjectUrl}
                      alt={displayName || 'Matrix User Avatar'}
                      className="w-5 h-5 rounded-full object-cover border border-neutral-300 dark:border-neutral-700"
                    />
                    <span className="font-mono text-xs max-w-[120px] truncate">
                      {displayName}
                    </span>
                  </>
                ) : (
                  <>
                    <div className="w-5 h-5 rounded-full bg-emerald-500 dark:bg-emerald-600 text-white flex items-center justify-center text-[10px] font-bold uppercase font-mono">
                      {(displayName || 'U')[0]}
                    </div>
                    <span className="font-mono text-xs max-w-[120px] truncate">
                      {displayName || 'Authenticated'}
                    </span>
                  </>
                )
              ) : (
                <>
                  <LogIn className="w-3.5 h-3.5 text-neutral-500 dark:text-neutral-400" />
                  <span>Login / Token</span>
                </>
              )}
            </button>

            {/* Timezone & currency preferences. Hidden while signed out —
                settings are per-user and there is no user yet. */}
            {userToken && (
              <button
                onClick={onOpenSettings}
                aria-label="Preferences"
                title="Timezone & currency"
                className="inline-flex items-center justify-center w-8 h-8 rounded-full bg-neutral-100 dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-neutral-600 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 transition cursor-pointer"
              >
                <Settings className="w-3.5 h-3.5" />
              </button>
            )}

            {/* Theme Toggle Component at the far end */}
            <ThemeToggle currentTheme={currentTheme} onThemeChange={onThemeChange} />
          </div>
        </div>
      </header>

      {/* Main Container */}
      <main className="flex-1 max-w-6xl w-full mx-auto px-6 py-8 space-y-8">
        {/* Stat Strip */}
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-4">
          <div className="p-4 rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 space-y-1">
            <div className="flex items-center justify-between text-neutral-500 dark:text-neutral-400 text-xs font-mono">
              <span>TOTAL TASKS</span>
              <CheckSquare className="w-3.5 h-3.5 text-neutral-400 dark:text-neutral-500" />
            </div>
            <div className="text-2xl font-bold font-mono tracking-tight text-neutral-900 dark:text-neutral-100">
              {totalTodos}
            </div>
            <div className="text-[11px] text-neutral-500 dark:text-neutral-500 font-mono">Managed via Matrix / API</div>
          </div>

          <div className="p-4 rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 space-y-1">
            <div className="flex items-center justify-between text-neutral-500 dark:text-neutral-400 text-xs font-mono">
              <span>ACTIVE REMINDERS</span>
              <Clock className="w-3.5 h-3.5 text-neutral-400 dark:text-neutral-500" />
            </div>
            <div className="text-2xl font-bold font-mono tracking-tight text-neutral-900 dark:text-neutral-100">
              {activeReminders}
            </div>
            <div className="text-[11px] text-neutral-500 dark:text-neutral-500 font-mono">Pending execution</div>
          </div>

          <div className="p-4 rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 space-y-1">
            <div className="flex items-center justify-between text-neutral-500 dark:text-neutral-400 text-xs font-mono">
              <span>LIVE EVENTS</span>
              <Radio className="w-3.5 h-3.5 text-neutral-400 dark:text-neutral-500" />
            </div>
            <div className="text-2xl font-bold font-mono tracking-tight text-neutral-900 dark:text-neutral-100">
              {events.length}
            </div>
            <div className="text-[11px] text-neutral-500 dark:text-neutral-500 font-mono">Streamed via SSE</div>
          </div>

          <div className="p-4 rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 space-y-1">
            <div className="flex items-center justify-between text-neutral-500 dark:text-neutral-400 text-xs font-mono">
              <span>OPENAPI API SPEC</span>
              <Shield className="w-3.5 h-3.5 text-neutral-400 dark:text-neutral-500" />
            </div>
            <div className="text-2xl font-bold font-mono tracking-tight text-neutral-900 dark:text-neutral-100">
              v3.0
            </div>
            <div className="text-[11px] text-neutral-500 dark:text-neutral-500 font-mono">utoipa + openapi-ts</div>
          </div>
        </div>

        {/* Dynamic Navigation Tabs */}
        <div className="flex items-center gap-1 border-b border-neutral-200 dark:border-neutral-800 pb-0">
          {modules.map((mod) => (
            <button
              key={mod.id}
              onClick={() => onSelectModule(mod.id)}
              className={`px-4 py-2.5 text-sm font-medium transition border-b-2 -mb-px ${
                activeModuleId === mod.id
                  ? 'border-neutral-900 dark:border-white text-neutral-900 dark:text-white font-semibold'
                  : 'border-transparent text-neutral-500 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200'
              }`}
            >
              {mod.navTab?.label || mod.name}
            </button>
          ))}
        </div>

        {/* Active Module View */}
        <div className="pt-2">
          {activeModule && (
            <activeModule.MainView
              {...(activeModule.id === 'core' ? { events, isConnected } : {})}
            />
          )}
        </div>
      </main>

      {/* Footer */}
      <footer className="border-t border-neutral-200 dark:border-neutral-800 py-6 px-6 text-xs text-neutral-500 font-mono">
        <div className="max-w-6xl mx-auto flex items-center justify-between">
          <div className="flex items-center gap-2">
            <span>Matrix Halo Bot — Modular Plugin System & REST API</span>
          </div>
          <span>Confidential & Proprietary</span>
        </div>
      </footer>
    </div>
  );
};
