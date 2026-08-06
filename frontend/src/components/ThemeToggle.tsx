import React from 'react';
import { Theme } from '../core/theme';
import { Sun, Moon, Laptop } from 'lucide-react';

interface ThemeToggleProps {
  currentTheme: Theme;
  onThemeChange: (theme: Theme) => void;
}

export const ThemeToggle: React.FC<ThemeToggleProps> = ({ currentTheme, onThemeChange }) => {
  const options: { theme: Theme; icon: React.ReactNode; label: string }[] = [
    { theme: 'light', icon: <Sun className="w-3.5 h-3.5" />, label: 'Light' },
    { theme: 'dark', icon: <Moon className="w-3.5 h-3.5" />, label: 'Dark' },
    { theme: 'system', icon: <Laptop className="w-3.5 h-3.5" />, label: 'System' },
  ];

  return (
    <div className="flex items-center bg-neutral-100 dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded-lg p-0.5 text-xs font-mono">
      {options.map(({ theme, icon, label }) => (
        <button
          key={theme}
          onClick={() => onThemeChange(theme)}
          title={`Switch to ${label} theme`}
          className={`flex items-center gap-1.5 px-2 py-1 rounded-md transition ${
            currentTheme === theme
              ? 'bg-white dark:bg-neutral-800 text-black dark:text-white font-medium shadow-xs'
              : 'text-neutral-500 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200'
          }`}
        >
          {icon}
          <span className="hidden sm:inline">{label}</span>
        </button>
      ))}
    </div>
  );
};
