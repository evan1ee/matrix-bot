export type Theme = 'dark' | 'light';

const THEME_STORAGE_KEY = 'matrix_bot_theme';

export function getStoredTheme(): Theme {
  const saved = localStorage.getItem(THEME_STORAGE_KEY) as Theme;
  if (saved === 'dark' || saved === 'light') {
    return saved;
  }
  return 'dark';
}

export function setStoredTheme(theme: Theme) {
  localStorage.setItem(THEME_STORAGE_KEY, theme);
  applyTheme(theme);
}

export function applyTheme(theme: Theme) {
  const root = document.documentElement;
  const isDark = theme === 'dark';

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
