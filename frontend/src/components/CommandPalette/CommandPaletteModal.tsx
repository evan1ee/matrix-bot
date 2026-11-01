import React, { useEffect, useId, useMemo, useRef, useState } from 'react';
import { Search } from 'lucide-react';
import { useKnowledgeSearch } from '../../modules/knowledge/api';
import { rankByQuery } from './paletteMatch';
import {
  buildAskAiAction,
  buildSearchActions,
  type PaletteAction,
  type PaletteContext,
} from './paletteRegistry';

const SEARCH_DEBOUNCE_MS = 200;
const SEARCH_LIMIT = 6;

interface CommandPaletteModalProps {
  onClose: () => void;
  staticActions: PaletteAction[];
  context: PaletteContext;
}

function useDebounced(value: string, delay: number): string {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value), delay);
    return () => clearTimeout(timer);
  }, [value, delay]);
  return debounced;
}

/** Rendered only while open, so state resets and focus is captured on each open. */
export const CommandPaletteModal: React.FC<CommandPaletteModalProps> = ({
  onClose,
  staticActions,
  context,
}) => {
  const [query, setQuery] = useState('');
  const [activeIndex, setActiveIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listId = useId();

  const trimmed = query.trim();
  const debounced = useDebounced(trimmed, SEARCH_DEBOUNCE_MS);
  const search = useKnowledgeSearch(debounced, undefined, SEARCH_LIMIT);

  // Restore whatever had focus before the palette opened.
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    inputRef.current?.focus();
    return () => previous?.focus?.();
  }, []);

  const actions = useMemo(() => {
    const local = rankByQuery(trimmed, staticActions, (a) => [a.title, a.subtitle ?? '', ...(a.keywords ?? [])]);
    if (trimmed === '') return local;
    const found = debounced === trimmed ? buildSearchActions(search.data?.items ?? [], context) : [];
    return [...local, ...found, buildAskAiAction(trimmed)];
  }, [trimmed, debounced, staticActions, search.data, context]);

  useEffect(() => setActiveIndex(0), [trimmed]);
  const active = Math.min(activeIndex, actions.length - 1);

  const execute = (action: PaletteAction | undefined) => {
    if (!action) return;
    onClose();
    action.run();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      onClose();
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      setActiveIndex((i) => (actions.length === 0 ? 0 : (i + 1) % actions.length));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setActiveIndex((i) => (actions.length === 0 ? 0 : (i - 1 + actions.length) % actions.length));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      execute(actions[active]);
    } else if (e.key === 'Tab') {
      // The input is the only tab stop; options are reached by arrow keys.
      e.preventDefault();
    }
  };

  const optionId = (index: number) => `${listId}-option-${index}`;

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center p-4 pt-[15vh] bg-black/60"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
      role="presentation"
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        onKeyDown={onKeyDown}
        className="w-full max-w-xl rounded-lg border border-[var(--color-border-subtle)] bg-[var(--color-canvas)] shadow-xl overflow-hidden"
      >
        <div className="flex items-center gap-2 px-4 border-b border-[var(--color-border-subtle)]">
          <Search className="w-4 h-4 shrink-0 text-[var(--color-text-tertiary)]" aria-hidden="true" />
          <input
            ref={inputRef}
            role="combobox"
            aria-expanded="true"
            aria-controls={listId}
            aria-activedescendant={actions.length > 0 ? optionId(active) : undefined}
            aria-autocomplete="list"
            aria-label="Search commands, pages and notes"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Jump to a page, run an action, or search…"
            className="flex-1 py-3 bg-transparent text-sm text-[var(--color-text-primary)] placeholder:text-[var(--color-text-tertiary)] outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-text-primary)]"
          />
        </div>
        <ul id={listId} role="listbox" aria-label="Results" className="max-h-80 overflow-y-auto py-1">
          {actions.length === 0 && (
            <li role="presentation" className="px-4 py-6 text-center text-xs text-[var(--color-text-tertiary)]">
              No matches
            </li>
          )}
          {actions.map((action, index) => {
            const Icon = action.icon;
            const showHeader = index === 0 || actions[index - 1].category !== action.category;
            return (
              <React.Fragment key={action.id}>
                {showHeader && (
                  <li role="presentation" className="px-4 pt-2 pb-1 text-xs font-medium text-[var(--color-text-tertiary)]">
                    {action.category}
                  </li>
                )}
                <li
                  id={optionId(index)}
                  role="option"
                  aria-selected={index === active}
                  onMouseMove={() => setActiveIndex(index)}
                  onClick={() => execute(action)}
                  className={`flex items-center gap-3 px-4 py-2 text-[13px] cursor-pointer transition-colors duration-150 ${
                    index === active
                      ? 'bg-[var(--color-surface-hover)] text-[var(--color-text-primary)]'
                      : 'text-[var(--color-text-secondary)]'
                  }`}
                >
                  <Icon className="w-4 h-4 shrink-0" aria-hidden="true" />
                  <span className="truncate font-medium">{action.title}</span>
                  {action.subtitle && (
                    <span className="truncate ml-auto text-xs text-[var(--color-text-tertiary)]">{action.subtitle}</span>
                  )}
                </li>
              </React.Fragment>
            );
          })}
        </ul>
      </div>
    </div>
  );
};
