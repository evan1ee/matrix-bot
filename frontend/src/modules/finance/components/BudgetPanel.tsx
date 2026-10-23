import React, { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ChevronLeft, ChevronRight, Plus } from 'lucide-react';
import { fetchApi } from '../../../core/apiClient';
import { formatInZone, todayInZone, useSettings } from '../../../core/settings';
import { BudgetModal, type BudgetModalSubmit } from './BudgetModal';
import { BudgetRow } from './BudgetRow';
import type { BudgetStatus, Category } from '../types';

interface BudgetPanelProps {
  categories: Category[];
  defaultCurrency: string;
}

/** Shifts a `YYYY-MM` key by whole months. */
function shiftMonth(month: string, delta: number): string {
  const [y, m] = month.split('-').map(Number);
  const index = y * 12 + (m - 1) + delta;
  return `${Math.floor(index / 12)}-${String((index % 12) + 1).padStart(2, '0')}`;
}

const NAV_BUTTON =
  'p-1.5 rounded text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition-colors duration-150 cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed focus-visible:outline-2 focus-visible:outline-neutral-500';

export const BudgetPanel: React.FC<BudgetPanelProps> = ({ categories, defaultCurrency }) => {
  const queryClient = useQueryClient();
  const { timezone } = useSettings();
  const currentMonth = todayInZone(timezone).slice(0, 7);
  const [selected, setSelected] = useState<string | null>(null);
  const month = selected ?? currentMonth;
  const [modal, setModal] = useState<{ editing: BudgetStatus | null } | null>(null);

  const { data: budgets, isLoading, error } = useQuery<BudgetStatus[]>({
    queryKey: ['finance-budgets', month],
    queryFn: () => fetchApi<BudgetStatus[]>(`/finance/budgets?month=${month}`),
  });

  const refresh = () => queryClient.invalidateQueries({ queryKey: ['finance-budgets'] });

  const saveMutation = useMutation({
    mutationFn: (s: BudgetModalSubmit) =>
      s.kind === 'create'
        ? fetchApi<BudgetStatus>('/finance/budgets', {
            method: 'POST',
            body: JSON.stringify(s.body),
          })
        : fetchApi<BudgetStatus>(`/finance/budgets/${s.id}`, {
            method: 'PUT',
            body: JSON.stringify(s.body),
          }),
    onSuccess: refresh,
  });

  const removeMutation = useMutation({
    mutationFn: (id: number) => fetchApi(`/finance/budgets/${id}`, { method: 'DELETE' }),
    onSuccess: refresh,
  });

  const handleSubmit = async (s: BudgetModalSubmit) => {
    await saveMutation.mutateAsync(s);
    setModal(null);
  };

  // Mid-month noon UTC is the same calendar month in every zone.
  const monthLabel = formatInZone(`${month}-15T12:00:00Z`, timezone, {
    month: 'long',
    year: 'numeric',
  });

  return (
    <section className="space-y-3" aria-labelledby="budgets-heading">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-1">
          <h2
            id="budgets-heading"
            className="text-sm font-semibold text-neutral-900 dark:text-neutral-100 font-sans"
          >
            Budgets
          </h2>
          <button
            type="button"
            aria-label="Previous month"
            onClick={() => setSelected(shiftMonth(month, -1))}
            className={NAV_BUTTON}
          >
            <ChevronLeft className="w-4 h-4" />
          </button>
          <span data-testid="budget-month" className="text-xs font-sans text-neutral-600 dark:text-neutral-400 min-w-28 text-center">
            {monthLabel}
          </span>
          <button
            type="button"
            aria-label="Next month"
            disabled={month >= currentMonth}
            onClick={() => setSelected(shiftMonth(month, 1))}
            className={NAV_BUTTON}
          >
            <ChevronRight className="w-4 h-4" />
          </button>
        </div>
        <button
          type="button"
          onClick={() => setModal({ editing: null })}
          className="h-8 px-3 rounded-md flex items-center gap-1.5 text-xs font-medium border border-neutral-300 dark:border-neutral-800 text-neutral-700 dark:text-neutral-300 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition-colors duration-150 cursor-pointer focus-visible:outline-2 focus-visible:outline-neutral-500"
        >
          <Plus className="w-3.5 h-3.5" />
          <span>Add budget</span>
        </button>
      </div>

      {error ? (
        <p className="text-xs text-red-600 dark:text-red-400 font-sans">{error.message}</p>
      ) : isLoading ? (
        <p className="text-xs text-neutral-500 font-sans">Loading budgets…</p>
      ) : budgets && budgets.length > 0 ? (
        <ul className="divide-y divide-neutral-200 dark:divide-neutral-800">
          {budgets.map((b) => (
            <BudgetRow
              key={b.budget.id}
              status={b}
              onEdit={() => setModal({ editing: b })}
              onRemove={() => removeMutation.mutate(b.budget.id)}
            />
          ))}
        </ul>
      ) : (
        <p className="text-xs text-neutral-500 font-sans">
          No budgets yet. Add one to track a monthly spending limit.
        </p>
      )}

      {modal && (
        <BudgetModal
          editing={modal.editing}
          categories={categories}
          defaultCurrency={defaultCurrency}
          onClose={() => setModal(null)}
          onSubmit={handleSubmit}
        />
      )}
    </section>
  );
};
