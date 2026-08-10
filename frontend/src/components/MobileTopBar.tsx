import React from 'react';
import { Menu, Bot } from 'lucide-react';

interface MobileTopBarProps {
  moduleLabel: string;
  isConnected: boolean;
  userToken: string | null;
  onOpenSidebar: () => void;
}

/**
 * Slim top bar visible only on mobile (< 768px). Shows a hamburger to
 * toggle the sidebar overlay, the active module name, and a status dot.
 */
export const MobileTopBar: React.FC<MobileTopBarProps> = ({
  moduleLabel,
  isConnected,
  userToken,
  onOpenSidebar,
}) => {
  return (
    <header className="md:hidden flex items-center justify-between px-4 py-3 border-b border-[var(--color-border-subtle)] bg-[var(--color-surface-subtle)]">
      <button
        onClick={onOpenSidebar}
        aria-label="Open navigation"
        className="p-1.5 rounded-md hover:bg-[var(--color-surface-hover)] transition duration-150 cursor-pointer"
      >
        <Menu className="w-5 h-5 text-[var(--color-text-primary)]" />
      </button>

      <div className="flex items-center gap-2">
        <Bot className="w-4 h-4 text-[var(--color-text-secondary)]" />
        <span className="text-sm font-semibold text-[var(--color-text-primary)] tracking-tight">
          {moduleLabel}
        </span>
      </div>

      <div className="flex items-center">
        <span
          className={`w-2 h-2 rounded-full ${
            !userToken
              ? 'bg-neutral-400 dark:bg-neutral-600'
              : isConnected
              ? 'bg-emerald-500 dark:bg-emerald-400 animate-pulse'
              : 'bg-amber-500 dark:bg-amber-400'
          }`}
          title={!userToken ? 'Not authenticated' : isConnected ? 'Connected' : 'Reconnecting'}
        />
      </div>
    </header>
  );
};
