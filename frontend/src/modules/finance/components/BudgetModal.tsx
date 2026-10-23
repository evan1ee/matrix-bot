import React, { useEffect, useState } from 'react';
import { X } from 'lucide-react';
import { CustomSelect } from '../../../components/CustomSelect';
import { currencyOptionsFor, formatMinor } from '../utils';
import type { BudgetStatus, Category, CreateBudgetBody, UpdateBudgetBody } from '../types';

export type BudgetModalSubmit =
  | { kind: 'create'; body: CreateBudgetBody }
  | { kind: 'update'; id: number; body: UpdateBudgetBody };

interface BudgetModalProps {
  /** The budget being edited, or `null` when adding a new one. */
  editing: BudgetStatus | null;
  categories: Category[];
  defaultCurrency: string;
  onClose: () => void;
  /** Rejects with the server's message; the modal shows it inline. */
  onSubmit: (submit: BudgetModalSubmit) => Promise<void>;
}

const OVERALL = 'overall';
const LABEL = 'text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans';

export const BudgetModal: React.FC<BudgetModalProps> = ({
  editing,
  categories,
  defaultCurrency,
  onClose,
  onSubmit,
}) => {
  const [scope, setScope] = useState(
    editing?.budget.category_id != null ? String(editing.budget.category_id) : OVERALL
  );
  const [currency, setCurrency] = useState(editing?.budget.currency ?? defaultCurrency);
  const [limit, setLimit] = useState(
    editing ? formatMinor(editing.budget.limit_minor, editing.budget.currency_exponent) : ''
  );
  const [alerts, setAlerts] = useState(editing?.budget.alerts_enabled ?? true);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [onClose]);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    const trimmed = limit.trim();
    // The server is authoritative on precision; this only catches the obvious.
    if (!/^\d*\.?\d+$|^\d+\.$/.test(trimmed) || Number(trimmed) <= 0) {
      setError('Enter a limit greater than zero, such as 500 or 250.50.');
      return;
    }
    setError(null);
    setBusy(true);
    try {
      if (editing) {
        await onSubmit({
          kind: 'update',
          id: editing.budget.id,
          body: { limit: trimmed, alerts_enabled: alerts },
        });
      } else {
        await onSubmit({
          kind: 'create',
          body: {
            category_id: scope === OVERALL ? null : Number(scope),
            currency: currency.trim().toUpperCase(),
            limit: trimmed,
            alerts_enabled: alerts,
          },
        });
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Could not save the budget.');
    } finally {
      setBusy(false);
    }
  };

  const title = editing ? 'Edit budget' : 'Add budget';
  const scopeLabel = editing ? (editing.budget.category_name ?? 'Overall') : null;

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="budget-modal-title"
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60"
    >
      <div className="w-full max-w-md rounded-lg bg-white dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-6 shadow-xl space-y-5">
        <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3">
          <h3
            id="budget-modal-title"
            className="text-sm font-semibold text-neutral-900 dark:text-neutral-100 font-sans"
          >
            {title}
          </h3>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close dialog"
            className="p-1 rounded text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-200 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition-colors duration-150 cursor-pointer focus-visible:outline-2 focus-visible:outline-neutral-500"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {error && (
          <div
            role="alert"
            className="p-2.5 rounded text-xs bg-red-500/10 text-red-600 dark:text-red-400 border border-red-500/20 font-sans"
          >
            {error}
          </div>
        )}

        <form onSubmit={submit} className="space-y-4">
          {editing ? (
            <p className="text-xs text-neutral-500 font-sans">
              {scopeLabel} · <span className="font-mono">{editing.budget.currency}</span>
            </p>
          ) : (
            <>
              <label className="flex flex-col gap-1">
                <span className={LABEL}>Scope</span>
                <CustomSelect
                  value={scope}
                  onChange={setScope}
                  options={[
                    { value: OVERALL, label: 'Overall' },
                    ...categories.map((c) => ({ value: String(c.id), label: c.name })),
                  ]}
                />
              </label>
              <label className="flex flex-col gap-1">
                <span className={LABEL}>Currency</span>
                <CustomSelect
                  value={currency}
                  onChange={setCurrency}
                  mono
                  options={currencyOptionsFor(currency)}
                />
              </label>
            </>
          )}

          <label className="flex flex-col gap-1">
            <span className={LABEL}>Monthly limit</span>
            <input
              type="text"
              inputMode="decimal"
              autoFocus
              placeholder="500"
              value={limit}
              onChange={(e) => setLimit(e.target.value)}
              className="h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus-visible:outline-2 focus-visible:outline-neutral-500 font-mono"
            />
          </label>

          <label className="flex items-center gap-2 cursor-pointer">
            <input
              type="checkbox"
              checked={alerts}
              onChange={(e) => setAlerts(e.target.checked)}
              className="w-4 h-4 accent-neutral-900 dark:accent-neutral-100"
            />
            <span className={LABEL}>Alert me at 80 % and 100 %</span>
          </label>

          <div className="flex justify-end gap-2 pt-1">
            <button
              type="button"
              onClick={onClose}
              className="h-9 px-3 rounded-md text-xs font-medium text-neutral-600 dark:text-neutral-400 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition-colors duration-150 cursor-pointer focus-visible:outline-2 focus-visible:outline-neutral-500"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={busy}
              className="h-9 px-4 rounded-md text-xs font-medium bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900 hover:bg-neutral-700 dark:hover:bg-neutral-300 transition-colors duration-150 cursor-pointer disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-neutral-500"
            >
              {editing ? 'Save' : 'Add'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
