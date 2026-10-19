import React, { useState } from 'react';
import { ArrowDown, ArrowUp, Plus, X } from 'lucide-react';
import { fetchApi } from '../../../core/apiClient';
import { TodoDetail, TodoStep } from '../types';

export interface StepListProps {
  todoId: number;
  steps: TodoStep[];
  /** Receives the fresh detail returned by the server after every successful change. */
  onChange: (detail: TodoDetail) => void;
}

const iconButton =
  'p-1 rounded text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-200 dark:hover:bg-neutral-900 transition-colors duration-150 cursor-pointer disabled:opacity-30 disabled:cursor-default focus-visible:outline focus-visible:outline-2 focus-visible:outline-neutral-500';

/** Inline checklist: add, toggle, remove and reorder with up/down buttons. */
export const StepList: React.FC<StepListProps> = ({ todoId, steps, onChange }) => {
  const [draft, setDraft] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const base = `/todos/${todoId}/steps`;

  const run = async (request: Promise<TodoDetail>): Promise<boolean> => {
    setBusy(true);
    setError(null);
    try {
      onChange(await request);
      return true;
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to update the checklist.');
      return false;
    } finally {
      setBusy(false);
    }
  };

  const add = async (e: React.FormEvent) => {
    e.preventDefault();
    const text = draft.trim();
    if (!text) return;
    const ok = await run(
      fetchApi<TodoDetail>(base, { method: 'POST', body: JSON.stringify({ text }) })
    );
    if (ok) setDraft('');
  };

  const toggle = (step: TodoStep) =>
    run(
      fetchApi<TodoDetail>(`${base}/${step.id}`, {
        method: 'PATCH',
        body: JSON.stringify({ done: !step.done }),
      })
    );

  const remove = (step: TodoStep) =>
    run(fetchApi<TodoDetail>(`${base}/${step.id}`, { method: 'DELETE' }));

  const move = (index: number, delta: -1 | 1) => {
    const ids = steps.map((s) => s.id);
    [ids[index], ids[index + delta]] = [ids[index + delta], ids[index]];
    return run(
      fetchApi<TodoDetail>(`${base}/order`, {
        method: 'PUT',
        body: JSON.stringify({ step_ids: ids }),
      })
    );
  };

  return (
    <div className="pl-10 pr-3 pb-3 space-y-2" data-testid="step-list">
      <ul className="space-y-1">
        {steps.map((step, i) => (
          <li key={step.id} className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={step.done}
              disabled={busy}
              aria-label={step.text}
              onChange={() => toggle(step)}
              className="shrink-0 cursor-pointer accent-neutral-900 dark:accent-neutral-100"
            />
            <span
              className={`flex-1 truncate ${
                step.done
                  ? 'line-through text-neutral-400 dark:text-neutral-500'
                  : 'text-neutral-800 dark:text-neutral-200'
              }`}
            >
              {step.text}
            </span>
            <button
              type="button"
              className={iconButton}
              aria-label={`Move ${step.text} up`}
              disabled={busy || i === 0}
              onClick={() => move(i, -1)}
            >
              <ArrowUp className="w-3.5 h-3.5" />
            </button>
            <button
              type="button"
              className={iconButton}
              aria-label={`Move ${step.text} down`}
              disabled={busy || i === steps.length - 1}
              onClick={() => move(i, 1)}
            >
              <ArrowDown className="w-3.5 h-3.5" />
            </button>
            <button
              type="button"
              className={iconButton}
              aria-label={`Remove ${step.text}`}
              disabled={busy}
              onClick={() => remove(step)}
            >
              <X className="w-3.5 h-3.5" />
            </button>
          </li>
        ))}
      </ul>

      <form onSubmit={add} className="flex items-center gap-2">
        <input
          type="text"
          aria-label="New step"
          placeholder="Add a step"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          className="flex-1 h-8 px-2 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 focus:outline-none focus:border-neutral-500 transition-colors duration-150 font-sans"
        />
        <button type="submit" disabled={busy} className={iconButton} aria-label="Add step">
          <Plus className="w-4 h-4" />
        </button>
      </form>

      {error && (
        <p role="alert" className="text-xs font-mono text-rose-600 dark:text-rose-400">
          {error}
        </p>
      )}
    </div>
  );
};
