export type Theme = 'dark' | 'light' | 'system';

const THEME_STORAGE_KEY = 'matrix_bot_theme';

export function getStoredTheme(): Theme {
  const saved = localStorage.getItem(THEME_STORAGE_KEY) as Theme;
  if (saved === 'dark' || saved === 'light' || saved === 'system') {
    return saved;
  }
  return 'dark'; // Default to Vercel dark mode
}

export function setStoredTheme(theme: Theme) {
  localStorage.setItem(THEME_STORAGE_KEY, theme);
  applyTheme(theme);
}

export function applyTheme(theme: Theme) {
  const root = document.documentElement;
  
  let isDark = false;
  if (theme === 'system') {
    isDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
  } else {
    isDark = theme === 'dark';
  }

  if (isDark) {
    root.classList.add('dark');
    root.classList.remove('light');
    root.style.colorScheme = 'dark';
  } else {
    root.classList.add('light');
    root.classList.remove('dark');
    root.style.colorScheme = 'light';
  }
}
