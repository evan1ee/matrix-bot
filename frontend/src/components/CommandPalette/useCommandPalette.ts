import { useCallback, useEffect, useState } from 'react';

export interface CommandPaletteState {
  isOpen: boolean;
  open: () => void;
  close: () => void;
}

/** Owns the open state and the Cmd+K / Ctrl+K toggle. */
export function useCommandPalette(enabled: boolean): CommandPaletteState {
  const [isOpen, setIsOpen] = useState(false);

  useEffect(() => {
    if (!enabled) {
      setIsOpen(false);
      return;
    }
    const onKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.altKey && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        setIsOpen((open) => !open);
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [enabled]);

  const open = useCallback(() => setIsOpen(true), []);
  const close = useCallback(() => setIsOpen(false), []);
  return { isOpen, open, close };
}
