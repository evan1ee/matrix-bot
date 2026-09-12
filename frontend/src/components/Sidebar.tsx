import { SessionUser } from '../core/session';
import React, { useEffect, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { fetchApi } from '../core/apiClient';
import { Theme } from '../core/theme';
import {
  Bot, ChevronLeft, LayoutDashboard, CheckSquare, Clock, Bookmark,
  Wallet, TrendingUp, Receipt, Webhook, Sparkles, Compass, Zap, Hourglass, NotebookPen, Settings, LogIn, Sun, Moon, LucideIcon
} from 'lucide-react';

interface SidebarProps {
  activeModuleId: string;
  onSelectModule: (id: string) => void;
  isConnected: boolean;
  user: SessionUser | null;
  onOpenAuth: () => void;
  onOpenSettings: () => void;
  currentTheme: Theme;
  onThemeChange: (theme: Theme) => void;
  isCollapsed: boolean;
  onToggleCollapse: () => void;
  isMobileOpen: boolean;
  onCloseMobile: () => void;
}

interface UserProfileResponse {
  username: string;
  display_name?: string | null;
  avatar_url?: string | null;
}

type NavItem = { id: string; label: string; icon: LucideIcon };
type NavGroup = { title: string; items: NavItem[] };

const NAV_GROUPS: NavGroup[] = [
  {
    title: 'Productivity',
    items: [
      { id: 'dashboard', label: 'Dashboard', icon: LayoutDashboard },
      { id: 'todo', label: 'Tasks', icon: CheckSquare },
      { id: 'reminder', label: 'Reminders', icon: Clock },
      { id: 'bookmarks', label: 'Bookmarks', icon: Bookmark },
      { id: 'habits', label: 'Habits', icon: Zap },
      { id: 'ai', label: 'AI Assistant', icon: Sparkles },
    ],
  },
  {
    title: 'Finance',
    items: [
      { id: 'finance', label: 'Expenses', icon: Wallet },
      { id: 'portfolio', label: 'Portfolio', icon: TrendingUp },
      { id: 'recurring_bills', label: 'Bills', icon: Receipt },
    ],
  },
  {
    title: 'Lifestyle',
    items: [
      { id: 'journal', label: 'Daily Log', icon: NotebookPen },
      { id: 'footprints', label: 'Footprints', icon: Compass },
      { id: 'life', label: 'Life Grid', icon: Hourglass },
    ],
  },
  {
    title: 'Integrations',
    items: [
      { id: 'webhooks', label: 'Webhooks', icon: Webhook },
    ],
  },
];

export const Sidebar: React.FC<SidebarProps> = ({
  activeModuleId,
  onSelectModule,
  isConnected,
  user,
  onOpenAuth,
  onOpenSettings,
  currentTheme,
  onThemeChange,
  isCollapsed,
  onToggleCollapse,
  isMobileOpen,
  onCloseMobile,
}) => {
  const [avatarObjectUrl, setAvatarObjectUrl] = useState<string | null>(null);

  const avatarPath = user?.avatar_url || null;

  useEffect(() => {
    if (!avatarPath || !user) {
      setAvatarObjectUrl(null);
      return;
    }
    let cancelled = false;
    let objectUrl: string | null = null;

    // The media proxy is an authenticated endpoint; the session cookie is what
    // authenticates it now, so this is a plain fetch that opts into credentials.
    fetch(avatarPath, { credentials: 'include' })
      .then((res) => (res.ok ? res.blob() : Promise.reject(new Error('Avatar unavailable'))))
      .then((blob) => {
        if (cancelled) return;
        objectUrl = URL.createObjectURL(blob);
        setAvatarObjectUrl(objectUrl);
      })
      .catch(() => { if (!cancelled) setAvatarObjectUrl(null); });

    return () => {
      cancelled = true;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [avatarPath, user]);

  const handleNav = (id: string) => {
    onSelectModule(id);
    if (isMobileOpen) onCloseMobile();
  };

  const displayName = user?.display_name || user?.username || 'User';
  const isDark = currentTheme === 'dark';

  const statusLabel = !user ? 'Auth Required' : isConnected ? 'SSE Live' : 'Reconnecting';
  const statusColor = !user
    ? 'bg-neutral-400 dark:bg-neutral-600'
    : isConnected
    ? 'bg-emerald-500'
    : 'bg-amber-500';
  const statusDesc = !user
    ? 'Sign in to establish real-time event streaming.'
    : isConnected
    ? 'Real-time event stream is connected and receiving background updates.'
    : 'Attempting to re-establish connection to live event stream.';

  const renderDetailCard = () => (
    <div className="p-3 rounded-lg bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 shadow-xl text-left font-sans text-xs">
      <div className="flex items-center justify-between pb-2 border-b border-neutral-100 dark:border-neutral-800 mb-2">
        <div className="flex items-center gap-2">
          <span className={`w-2 h-2 rounded-full shrink-0 ${statusColor}`} />
          <span className="font-semibold text-neutral-900 dark:text-neutral-100 text-xs">
            {statusLabel}
          </span>
        </div>
        <span className="font-mono text-[10px] text-neutral-500 dark:text-neutral-400">
          SSE
        </span>
      </div>
      <div className="space-y-1.5 text-[11px]">
        <div className="flex items-center justify-between text-neutral-500 dark:text-neutral-400">
          <span>Protocol</span>
          <span className="font-mono text-[10px] text-neutral-700 dark:text-neutral-300">Server-Sent Events</span>
        </div>
        <div className="flex items-center justify-between text-neutral-500 dark:text-neutral-400">
          <span>Endpoint</span>
          <span className="font-mono text-[10px] text-neutral-700 dark:text-neutral-300">/api/v1/events</span>
        </div>
        <div className="flex items-center justify-between text-neutral-500 dark:text-neutral-400">
          <span>Sync Scope</span>
          <span className="text-neutral-700 dark:text-neutral-300">Tasks, Reminders, Bills, AI</span>
        </div>
      </div>
      <p className="mt-2 text-[11px] leading-relaxed text-neutral-500 dark:text-neutral-400 border-t border-neutral-100 dark:border-neutral-800 pt-2">
        {statusDesc}
      </p>
    </div>
  );

  /** Standalone connection status dot button for expanded mode */
  const renderConnectionStatus = () => (
    <div className="relative group flex items-center justify-center">
      <button
        type="button"
        data-testid="connection-status-dot"
        aria-label={`Connection status: ${statusLabel}`}
        title={`Connection status: ${statusLabel}`}
        className="p-1.5 rounded-md hover:bg-[var(--color-surface-hover)] text-[var(--color-text-tertiary)] hover:text-[var(--color-text-primary)] transition duration-150 cursor-pointer flex items-center justify-center"
      >
        <span className={`w-2 h-2 rounded-full ${statusColor}`} />
      </button>

      {/* Hover detail card positioned neatly above the bottom bar */}
      <div className="pointer-events-none absolute bottom-full right-[-52px] mb-2 w-[236px] opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 transition-opacity duration-150 z-50">
        {renderDetailCard()}
      </div>
    </div>
  );

  /** Inline theme toggle button */
  const renderThemeBtn = (compact: boolean) => (
    <button
      onClick={() => onThemeChange(isDark ? 'light' : 'dark')}
      title={`Switch to ${isDark ? 'light' : 'dark'} mode`}
      className={
        compact
          ? "w-9 h-9 flex items-center justify-center rounded-md hover:bg-[var(--color-surface-hover)] text-[var(--color-text-tertiary)] hover:text-[var(--color-text-primary)] transition duration-150 cursor-pointer"
          : "p-1.5 rounded-md hover:bg-[var(--color-surface-hover)] text-[var(--color-text-tertiary)] transition duration-150 cursor-pointer"
      }
    >
      {isDark ? <Moon className="w-4 h-4" /> : <Sun className="w-4 h-4 text-amber-500" />}
    </button>
  );

  /** Renders the sidebar body — used by both desktop and mobile wrappers. */
  const renderBody = (collapsed: boolean) => (
    <div className="flex flex-col h-full w-full">
      {/* ─── Zone 1: Workspace Header ─── */}
      {collapsed ? (
        <div className="flex items-center justify-center h-14 w-full shrink-0 border-b border-[var(--color-border-subtle)]">
          <button
            onClick={onToggleCollapse}
            className="w-8 h-8 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black flex items-center justify-center cursor-pointer hover:opacity-85 transition-opacity"
            title="Expand sidebar"
          >
            <Bot className="w-4 h-4" />
          </button>
        </div>
      ) : (
        <div className="flex items-center justify-between h-14 w-full shrink-0 px-4 border-b border-[var(--color-border-subtle)]">
          <div className="flex items-center gap-2.5 min-w-0">
            <div className="w-7 h-7 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black flex items-center justify-center shrink-0">
              <Bot className="w-3.5 h-3.5" />
            </div>
            <span className="text-[13px] font-semibold tracking-tight text-[var(--color-text-primary)] whitespace-nowrap">
              Matrix Halo Bot
            </span>
          </div>

          <button
            onClick={onToggleCollapse}
            className="hidden md:flex p-1 rounded-md hover:bg-[var(--color-surface-hover)] text-[var(--color-text-tertiary)] transition duration-150 cursor-pointer"
            title="Collapse sidebar"
          >
            <ChevronLeft className="w-3.5 h-3.5" />
          </button>
        </div>
      )}

      {/* ─── Zone 2: Navigation ─── */}
      <nav className={`flex-1 overflow-y-auto overflow-x-hidden w-full ${collapsed ? 'py-3' : 'py-4'}`}>
        {NAV_GROUPS.map((group, groupIdx) => (
          <div key={group.title} className={`w-full ${groupIdx > 0 ? (collapsed ? 'mt-3' : 'mt-5') : ''}`}>
            {/* Section header or divider */}
            {!collapsed ? (
              <h3 className="px-3 mb-1.5 text-xs font-medium text-[var(--color-text-tertiary)]">
                {group.title}
              </h3>
            ) : groupIdx > 0 ? (
              <div className="w-6 mx-auto mb-3 border-t border-[var(--color-border-subtle)]" />
            ) : null}

            <ul className={collapsed ? 'space-y-1.5 flex flex-col items-center w-full px-0' : 'space-y-0.5 px-3'}>
              {group.items.map((item) => {
                const Icon = item.icon;
                const isActive = activeModuleId === item.id;
                return (
                  <li key={item.id} className={collapsed ? 'w-full flex justify-center' : ''}>
                    <button
                      onClick={() => handleNav(item.id)}
                      title={collapsed ? item.label : undefined}
                      className={
                        collapsed
                          ? `w-9 h-9 flex items-center justify-center rounded-md text-[13px] font-medium transition-colors duration-150 cursor-pointer ${
                              isActive
                                ? 'bg-[var(--color-surface-hover)] text-[var(--color-text-primary)]'
                                : 'text-[var(--color-text-secondary)] hover:bg-[var(--color-surface-hover)] hover:text-[var(--color-text-primary)]'
                            }`
                          : `w-full flex items-center gap-3 px-3 py-[7px] rounded-md text-[13px] font-medium transition-colors duration-150 cursor-pointer ${
                              isActive
                                ? 'bg-[var(--color-surface-hover)] text-[var(--color-text-primary)]'
                                : 'text-[var(--color-text-secondary)] hover:bg-[var(--color-surface-hover)] hover:text-[var(--color-text-primary)]'
                            }`
                      }
                    >
                      {collapsed ? (
                        <Icon className="w-4 h-4 shrink-0" />
                      ) : (
                        <>
                          <div className="relative flex items-center justify-center w-4 h-4 shrink-0">
                            {isActive && (
                              <div className="absolute -left-[14px] w-[3px] h-4 rounded-r-full bg-[var(--color-text-primary)]" />
                            )}
                            <Icon className="w-4 h-4" />
                          </div>
                          <span className="truncate">{item.label}</span>
                        </>
                      )}
                    </button>
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
      </nav>

      {/* ─── Zone 3: User & Utilities ─── */}
      <div className={`border-t border-[var(--color-border-subtle)] shrink-0 w-full ${collapsed ? 'py-3 flex flex-col items-center gap-1.5' : 'px-3 py-3'}`}>
        {user ? (
          collapsed ? (
            <>
              {/* Avatar */}
              <button
                onClick={onOpenAuth}
                className="w-9 h-9 flex items-center justify-center rounded-md hover:bg-[var(--color-surface-hover)] transition duration-150 cursor-pointer"
                title={`Logged in as ${displayName}`}
              >
                <div className="w-6 h-6 rounded-full overflow-hidden shrink-0 bg-[var(--color-surface-hover)] flex items-center justify-center">
                  {avatarObjectUrl ? (
                    <img src={avatarObjectUrl} alt="Avatar" className="w-full h-full object-cover" />
                  ) : (
                    <div className="w-full h-full bg-emerald-500 dark:bg-emerald-600 text-white flex items-center justify-center text-[10px] font-bold uppercase font-mono">
                      {displayName[0]}
                    </div>
                  )}
                </div>
              </button>

              {/* Settings with connection dot in collapsed mode */}
              <div className="relative group flex items-center justify-center">
                <button
                  onClick={onOpenSettings}
                  className="w-9 h-9 flex items-center justify-center rounded-md hover:bg-[var(--color-surface-hover)] text-[var(--color-text-tertiary)] hover:text-[var(--color-text-primary)] transition duration-150 cursor-pointer relative"
                  title="Settings"
                >
                  <Settings className="w-4 h-4" />
                  <span
                    data-testid="connection-status-dot"
                    aria-label={`Connection status: ${statusLabel}`}
                    title={`Connection status: ${statusLabel}`}
                    className="absolute top-1.5 right-1.5 flex items-center justify-center"
                  >
                    <span className={`w-2 h-2 rounded-full ${statusColor}`} />
                  </span>
                </button>

                <div className="pointer-events-none absolute left-full bottom-0 ml-2 w-64 opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 transition-opacity duration-150 z-50">
                  {renderDetailCard()}
                </div>
              </div>

              {/* Theme */}
              {renderThemeBtn(true)}
            </>
          ) : (
            <div className="flex items-center gap-2 w-full">
              {/* Avatar */}
              <button
                onClick={onOpenAuth}
                className="flex items-center gap-2.5 rounded-md transition duration-150 cursor-pointer min-w-0 flex-1 px-2 py-1.5 hover:bg-[var(--color-surface-hover)]"
                title={`Logged in as ${displayName}`}
              >
                <div className="w-6 h-6 rounded-full overflow-hidden shrink-0 bg-[var(--color-surface-hover)] flex items-center justify-center">
                  {avatarObjectUrl ? (
                    <img src={avatarObjectUrl} alt="Avatar" className="w-full h-full object-cover" />
                  ) : (
                    <div className="w-full h-full bg-emerald-500 dark:bg-emerald-600 text-white flex items-center justify-center text-[10px] font-bold uppercase font-mono">
                      {displayName[0]}
                    </div>
                  )}
                </div>
                <span className="text-[13px] font-medium text-[var(--color-text-primary)] truncate min-w-0">
                  {displayName}
                </span>
              </button>

              {/* Utility buttons */}
              <div className="flex items-center shrink-0 gap-0.5">
                {renderConnectionStatus()}
                <button
                  onClick={onOpenSettings}
                  className="p-1.5 rounded-md hover:bg-[var(--color-surface-hover)] text-[var(--color-text-tertiary)] hover:text-[var(--color-text-primary)] transition duration-150 cursor-pointer"
                  title="Settings"
                >
                  <Settings className="w-3.5 h-3.5" />
                </button>
                {renderThemeBtn(false)}
              </div>
            </div>
          )
        ) : (
          collapsed ? (
            <>
              <button
                onClick={() => { onOpenAuth(); if (isMobileOpen) onCloseMobile(); }}
                className="w-9 h-9 flex items-center justify-center rounded-md text-[13px] font-medium text-[var(--color-text-secondary)] hover:bg-[var(--color-surface-hover)] transition duration-150 cursor-pointer"
                title="Sign In"
              >
                <LogIn className="w-4 h-4 shrink-0" />
              </button>
              {/* Connection status in collapsed logged out */}
              <div className="relative group flex items-center justify-center">
                <button
                  type="button"
                  data-testid="connection-status-dot"
                  aria-label={`Connection status: ${statusLabel}`}
                  title={`Connection status: ${statusLabel}`}
                  className="w-9 h-9 flex items-center justify-center rounded-md hover:bg-[var(--color-surface-hover)] text-[var(--color-text-tertiary)] transition duration-150 cursor-pointer"
                >
                  <span className={`w-2 h-2 rounded-full ${statusColor}`} />
                </button>
                <div className="pointer-events-none absolute left-full bottom-0 ml-2 w-64 opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 transition-opacity duration-150 z-50">
                  {renderDetailCard()}
                </div>
              </div>
              {renderThemeBtn(true)}
            </>
          ) : (
            <div className="flex items-center gap-1 w-full">
              <button
                onClick={() => { onOpenAuth(); if (isMobileOpen) onCloseMobile(); }}
                className="flex items-center gap-2 rounded-md text-[13px] font-medium text-[var(--color-text-secondary)] hover:bg-[var(--color-surface-hover)] transition duration-150 cursor-pointer flex-1 px-3 py-2"
                title="Sign In"
              >
                <LogIn className="w-4 h-4 shrink-0" />
                <span>Sign In</span>
              </button>
              {renderConnectionStatus()}
              {renderThemeBtn(false)}
            </div>
          )
        )}
      </div>
    </div>
  );

  return (
    <>
      {/* Mobile Overlay Backdrop */}
      {isMobileOpen && (
        <div
          className="fixed inset-0 bg-black/50 z-40 md:hidden sidebar-backdrop-enter"
          onClick={onCloseMobile}
        />
      )}

      {/* Mobile Sidebar Drawer */}
      <div
        className={`fixed top-0 left-0 h-full z-50 md:hidden transition-transform duration-200 ease-[cubic-bezier(0.32,0.72,0,1)] ${
          isMobileOpen ? 'translate-x-0' : '-translate-x-full'
        }`}
      >
        <div className="w-[260px] h-full bg-[var(--color-surface-subtle)] border-r border-[var(--color-border-subtle)] shadow-xl">
          {renderBody(false)}
        </div>
      </div>

      {/* Desktop Sidebar */}
      <aside
        className="hidden md:flex flex-col h-full shrink-0 bg-[var(--color-surface-subtle)] border-r border-[var(--color-border-subtle)] transition-all duration-200 ease-in-out"
        style={{ width: isCollapsed ? 'var(--sidebar-width-collapsed)' : 'var(--sidebar-width)' }}
      >
        {renderBody(isCollapsed)}
      </aside>
    </>
  );
};
