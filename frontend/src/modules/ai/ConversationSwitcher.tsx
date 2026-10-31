import React, { useEffect, useRef, useState } from 'react';
import { Check, ChevronDown, Pencil } from 'lucide-react';
import type { AiConversation } from './types';

const UNTITLED = 'New chat';

/** "3 minutes ago" style label for an ISO timestamp; empty when it cannot be read. */
export function relativeTime(iso: string, now: number = Date.now()): string {
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return '';
  const seconds = Math.round((then - now) / 1000);
  const units: [Intl.RelativeTimeFormatUnit, number][] = [
    ['day', 86400],
    ['hour', 3600],
    ['minute', 60],
  ];
  const formatter = new Intl.RelativeTimeFormat('en', { numeric: 'auto' });
  for (const [unit, size] of units) {
    if (Math.abs(seconds) >= size) return formatter.format(Math.round(seconds / size), unit);
  }
  return 'just now';
}

const BUTTON =
  'h-8 inline-flex items-center gap-1.5 px-3 text-xs font-medium rounded-md border border-neutral-200 dark:border-neutral-800 hover:bg-neutral-100 dark:hover:bg-neutral-900 text-neutral-700 dark:text-neutral-300 transition-colors duration-150 cursor-pointer disabled:opacity-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-neutral-900 dark:focus-visible:outline-neutral-100';

/** Lists the user's threads, switches between them and renames the active one. */
export const ConversationSwitcher: React.FC<{
  conversations: AiConversation[];
  activeId: string | null;
  onSelect: (id: string) => void;
  onRename: (id: string, title: string) => void;
  disabled?: boolean;
}> = ({ conversations, activeId, onSelect, onRename, disabled }) => {
  const [open, setOpen] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [draft, setDraft] = useState('');
  const rootRef = useRef<HTMLDivElement>(null);
  const active = conversations.find((c) => c.id === activeId) ?? null;

  useEffect(() => {
    if (!open) return;
    const onPointer = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    document.addEventListener('mousedown', onPointer);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onPointer);
      document.removeEventListener('keydown', onKey);
    };
  }, [open]);

  const startRename = () => {
    setDraft(active?.title ?? '');
    setRenaming(true);
    setOpen(false);
  };

  const commitRename = () => {
    const title = draft.trim();
    if (active && title && title !== active.title) onRename(active.id, title);
    setRenaming(false);
  };

  if (renaming && active) {
    return (
      <input
        autoFocus
        aria-label="Conversation title"
        value={draft}
        maxLength={100}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commitRename}
        onKeyDown={(e) => {
          if (e.key === 'Enter') commitRename();
          if (e.key === 'Escape') setRenaming(false);
        }}
        className="h-8 w-56 px-2.5 text-xs rounded-md border border-neutral-400 dark:border-neutral-600 bg-white dark:bg-neutral-900 text-neutral-900 dark:text-neutral-100 focus:outline-none"
      />
    );
  }

  return (
    <div ref={rootRef} className="relative flex items-center gap-1.5">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label="Switch conversation"
        className={`${BUTTON} max-w-56`}
      >
        <span className="truncate">{active?.title ?? UNTITLED}</span>
        <ChevronDown className="w-3.5 h-3.5 shrink-0 text-neutral-400" />
      </button>
      {active && (
        <button
          type="button"
          onClick={startRename}
          disabled={disabled}
          aria-label="Rename conversation"
          title="Rename conversation"
          className={`${BUTTON} px-2`}
        >
          <Pencil className="w-3.5 h-3.5 text-neutral-400" />
        </button>
      )}
      {open && (
        <ul
          role="listbox"
          aria-label="Conversations"
          className="absolute left-0 top-9 z-20 w-72 max-h-80 overflow-y-auto rounded-md border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 py-1 shadow-lg"
        >
          {conversations.map((c) => (
            <li key={c.id} role="option" aria-selected={c.id === activeId}>
              <button
                type="button"
                onClick={() => {
                  setOpen(false);
                  if (c.id !== activeId) onSelect(c.id);
                }}
                className="w-full flex items-center justify-between gap-3 px-3 py-2 text-left text-xs hover:bg-neutral-100 dark:hover:bg-neutral-800 transition-colors duration-150 cursor-pointer"
              >
                <span className="truncate text-neutral-900 dark:text-neutral-100">
                  {c.title ?? UNTITLED}
                </span>
                <span className="flex items-center gap-1.5 shrink-0 text-[10px] font-mono text-neutral-400">
                  {relativeTime(c.updated_at)}
                  {c.id === activeId && <Check className="w-3 h-3" />}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
};
