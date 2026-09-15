import { SessionUser } from '../core/session';
import React, { useState } from 'react';

import { registry } from '../core/registry';
import { SystemEvent } from '../core/types';
import { Theme } from '../core/theme';
import { Sidebar } from './Sidebar';
import { MobileTopBar } from './MobileTopBar';

interface AppShellProps {
  activeModuleId: string;
  onSelectModule: (id: string) => void;
  onQuickNavigate: (id: string) => void;
  focusModuleId: string | null;
  events: SystemEvent[];
  isConnected: boolean;
  onOpenAuth: () => void;
  onOpenSettings: () => void;
  user: SessionUser | null;
  currentTheme: Theme;
  onThemeChange: (theme: Theme) => void;
}

/** Module labels shown in the mobile top bar and content header. */
const MODULE_LABELS: Record<string, string> = {
  dashboard: 'Dashboard',
  todo: 'Tasks',
  reminder: 'Reminders',
  bookmarks: 'Bookmarks',
  portfolio: 'Portfolio',
  finance: 'Expenses',
  recurring_bills: 'Bills',
  footprints: 'Footprints',
  mood: 'Mood',
  time_blocks: 'Time Blocks',
  life: 'Life Grid',
  habits: 'Habits',
  webhooks: 'Webhooks',
  ai: 'AI Assistant',
  core: 'System Overview',
};

export const AppShell: React.FC<AppShellProps> = ({
  activeModuleId,
  onSelectModule,
  onQuickNavigate,
  focusModuleId,
  events,
  isConnected,
  onOpenAuth,
  onOpenSettings,
  user,
  currentTheme,
  onThemeChange,
}) => {
  const [isCollapsed, setIsCollapsed] = useState(false);
  const [isMobileOpen, setIsMobileOpen] = useState(false);

  const activeModule = registry.get(activeModuleId) || registry.getAll()[0];
  const moduleLabel = MODULE_LABELS[activeModuleId] || activeModule?.navTab?.label || activeModule?.name || '';

  return (
    <div className="flex h-full bg-[var(--color-canvas)] text-[var(--color-text-primary)] font-sans">
      {/* Sidebar (desktop: inline, mobile: overlay) */}
      <Sidebar
        activeModuleId={activeModuleId}
        onSelectModule={onSelectModule}
        isConnected={isConnected}
        user={user}
        onOpenAuth={onOpenAuth}
        onOpenSettings={onOpenSettings}
        currentTheme={currentTheme}
        onThemeChange={onThemeChange}
        isCollapsed={isCollapsed}
        onToggleCollapse={() => setIsCollapsed((prev) => !prev)}
        isMobileOpen={isMobileOpen}
        onCloseMobile={() => setIsMobileOpen(false)}
      />

      {/* Content Area */}
      <div className="flex-1 flex flex-col min-w-0 h-full">
        {/* Mobile Top Bar */}
        <MobileTopBar
          moduleLabel={moduleLabel}
          isConnected={isConnected}
          user={user}
          onOpenSidebar={() => setIsMobileOpen(true)}
        />

        {/* Content Header (desktop only) */}
        <header className="hidden md:flex items-center justify-between px-8 py-5 border-b border-[var(--color-border-subtle)] shrink-0">
          <h1 className="text-lg font-semibold tracking-tight text-[var(--color-text-primary)]">
            {moduleLabel}
          </h1>
        </header>

        {/* Module View */}
        <main className="flex-1 overflow-y-auto px-6 md:px-8 py-6 flex flex-col">
          {activeModule && (
            <activeModule.MainView
              {...(activeModule.id === 'core' ? { events, isConnected } : {})}
              onNavigate={onQuickNavigate}
              autoFocus={activeModule.id === focusModuleId}
            />
          )}
        </main>
      </div>
    </div>
  );
};
