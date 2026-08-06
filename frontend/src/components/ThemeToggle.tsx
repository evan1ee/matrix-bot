import React from 'react';
import { Theme } from '../core/theme';
import { Sun, Moon } from 'lucide-react';

interface ThemeToggleProps {
  currentTheme: Theme;
  onThemeChange: (theme: Theme) => void;
}

export const ThemeToggle: React.FC<ThemeToggleProps> = ({ currentTheme, onThemeChange }) => {
  const isDark = currentTheme === 'dark';

  const toggleTheme = () => {
    onThemeChange(isDark ? 'light' : 'dark');
  };

  return (
    <button
      onClick={toggleTheme}
      title={`Switch to ${isDark ? 'Light' : 'Dark'} mode`}
      className="p-2 rounded-full bg-neutral-100 dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-neutral-700 dark:text-neutral-300 hover:bg-neutral-200 dark:hover:bg-neutral-800 transition cursor-pointer shrink-0"
    >
      {isDark ? (
        <Moon className="w-4 h-4 text-neutral-200" />
      ) : (
        <Sun className="w-4 h-4 text-amber-500" />
      )}
    </button>
  );
};
