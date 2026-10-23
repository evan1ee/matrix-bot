import React, { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import { Wallet, BarChart2, Receipt } from 'lucide-react';
import { PublicTransparencyView } from './components/PublicTransparencyView';
import { NetWorthStatStrip } from './components/NetWorthStatStrip';
import { TransactionList } from './components/TransactionList';
import { BudgetPanel } from './components/BudgetPanel';
import { AddTransactionModal } from './components/AddTransactionModal';
import {
  nowInZone,
  useSettings,
  zonedInputToUtcIso,
} from '../../core/settings';
import {
  Direction,
  Transaction,
  Category,
  CurrencySummary,
  LedgerResponse,
} from './types';
import { formatMinor } from './utils';

function currentMonthRange(timezone: string): { from: string; to: string } {
  const now = nowInZone(timezone);
  const year = now.getFullYear();
  const month = now.getMonth();
  const monthStart = (y: number, m: number) =>
    zonedInputToUtcIso(`${y}-${String(m + 1).padStart(2, '0')}-01T00:00`, timezone) ??
    new Date(Date.UTC(y, m, 1)).toISOString();

  return {
    from: monthStart(year, month),
    to: month === 11 ? monthStart(year + 1, 0) : monthStart(year, month + 1),
  };
}

export const FinanceMainView: React.FC<{ autoFocus?: boolean }> = ({ autoFocus }) => {
  const queryClient = useQueryClient();
  const { timezone, currency: defaultCurrency } = useSettings();
  const [filter, setFilter] = useState<'all' | Direction>('all');
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [categoryError, setCategoryError] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<'transparency' | 'ledger'>('ledger');

  const directionQuery = filter !== 'all' ? `&direction=${filter}` : '';
  const { data: ledger, isLoading } = useQuery<LedgerResponse>({
    queryKey: ['transactions', filter],
    queryFn: () => fetchApi<LedgerResponse>(`/finance/transactions?limit=200${directionQuery}`),
  });

  const { data: categories } = useQuery<Category[]>({
    queryKey: ['finance-categories'],
    queryFn: () => fetchApi<Category[]>('/finance/categories'),
  });

  const month = currentMonthRange(timezone);
  const { data: summary } = useQuery<CurrencySummary[]>({
    queryKey: ['finance-summary', month.from, month.to],
    queryFn: () =>
      fetchApi<CurrencySummary[]>(
        `/finance/summary?from=${encodeURIComponent(month.from)}&to=${encodeURIComponent(month.to)}`
      ),
  });

  const invalidate = () => {
    queryClient.invalidateQueries({ queryKey: ['transactions'] });
    queryClient.invalidateQueries({ queryKey: ['finance-summary'] });
    queryClient.invalidateQueries({ queryKey: ['finance-analytics'] });
    queryClient.invalidateQueries({ queryKey: ['finance-budgets'] });
  };

  const createMutation = useMutation({
    mutationFn: (body: Record<string, unknown>) =>
      fetchApi<Transaction>('/finance/transactions', {
        method: 'POST',
        body: JSON.stringify(body),
      }),
    onSuccess: () => {
      setErrorMsg(null);
      invalidate();
    },
    onError: (err: Error) => setErrorMsg(err.message),
  });

  const deleteMutation = useMutation({
    mutationFn: (id: number) => fetchApi(`/finance/transactions/${id}`, { method: 'DELETE' }),
    onSuccess: invalidate,
  });

  const editMutation = useMutation({
    mutationFn: ({ id, ...body }: Record<string, unknown> & { id: number }) =>
      fetchApi<Transaction>(`/finance/transactions/${id}`, {
        method: 'PUT',
        body: JSON.stringify(body),
      }),
    onSuccess: invalidate,
  });

  const addCategoryMutation = useMutation({
    mutationFn: (name: string) =>
      fetchApi<Category>('/finance/categories', {
        method: 'POST',
        body: JSON.stringify({ name }),
      }),
    onSuccess: () => {
      setCategoryError(null);
      queryClient.invalidateQueries({ queryKey: ['finance-categories'] });
    },
    onError: (err: Error) => setCategoryError(err.message),
  });

  const deleteCategoryMutation = useMutation({
    mutationFn: (id: number) => fetchApi(`/finance/categories/${id}`, { method: 'DELETE' }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['finance-categories'] });
      invalidate();
    },
  });

  const items = ledger?.items ?? [];

  return (
    <div className="space-y-6">
      {/* Top View Selector */}
      <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3">
        <div className="flex items-center gap-1 bg-neutral-100 dark:bg-neutral-900 p-0.5 rounded-lg border border-neutral-200 dark:border-neutral-800 text-xs font-medium">
          <button
            type="button"
            onClick={() => setViewMode('transparency')}
            className={`px-3 py-1.5 rounded-md transition flex items-center gap-1.5 cursor-pointer ${
              viewMode === 'transparency'
                ? 'bg-white dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100 shadow-2xs font-semibold'
                : 'text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-200'
            }`}
          >
            <BarChart2 className="w-3.5 h-3.5" />
            <span>Fiscal Transparency</span>
          </button>
          <button
            type="button"
            onClick={() => setViewMode('ledger')}
            className={`px-3 py-1.5 rounded-md transition flex items-center gap-1.5 cursor-pointer ${
              viewMode === 'ledger'
                ? 'bg-white dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100 shadow-2xs font-semibold'
                : 'text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-200'
            }`}
          >
            <Receipt className="w-3.5 h-3.5" />
            <span>Ledger</span>
          </button>
        </div>
      </div>

      {viewMode === 'transparency' ? (
        <PublicTransparencyView currency={defaultCurrency} />
      ) : (
        <>
          {/* Unified cross-currency net worth header widget */}
          <NetWorthStatStrip />

          {/* Per-currency summary */}
          {summary && summary.length > 0 && (
            <div className="grid grid-cols-2 sm:grid-cols-4 gap-4" data-testid="finance-summary">
              {summary.map((s) => (
                <div
                  key={s.currency}
                  className="p-4 rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 space-y-1"
                >
                  <div className="flex items-center justify-between text-neutral-500 dark:text-neutral-400 text-xs font-medium font-sans">
                    <span><span className="font-mono">{s.currency}</span> this month</span>
                    <Wallet className="w-3.5 h-3.5 text-neutral-400" />
                  </div>
                  <div
                    className={`text-2xl font-semibold font-sans tabular-nums tracking-tight ${
                      s.net_minor < 0
                        ? 'text-rose-600 dark:text-rose-400'
                        : 'text-emerald-600 dark:text-emerald-400'
                    }`}
                  >
                    {formatMinor(s.net_minor, s.currency_exponent)}
                  </div>
                  <div className="text-[11px] text-neutral-500 font-mono">
                    in {formatMinor(s.income_minor, s.currency_exponent)} · out{' '}
                    {formatMinor(s.expense_minor, s.currency_exponent)}
                  </div>
                </div>
              ))}
            </div>
          )}

          {/* Entry form */}
          <AddTransactionModal
            categories={categories ?? []}
            defaultCurrency={defaultCurrency}
            onRecord={(body) => createMutation.mutate(body)}
            isRecording={createMutation.isPending}
            onAddCategory={(name) => addCategoryMutation.mutate(name)}
            onDeleteCategory={(id) => deleteCategoryMutation.mutate(id)}
            errorMsg={errorMsg}
            categoryError={categoryError}
            autoFocus={autoFocus}
          />

          <BudgetPanel categories={categories ?? []} defaultCurrency={defaultCurrency} />

          {/* Ledger */}
          <TransactionList
            items={items}
            isLoading={isLoading}
            categories={categories ?? []}
            filter={filter}
            onFilterChange={setFilter}
            onEdit={async (id, body) => {
              await editMutation.mutateAsync({ id, ...body });
            }}
            onDelete={(id) => deleteMutation.mutate(id)}
            isEditing={editMutation.isPending}
            timezone={timezone}
          />
        </>
      )}
    </div>
  );
};
