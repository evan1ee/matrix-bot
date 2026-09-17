import React, { useEffect, useRef, useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import { FrontendModule } from '../../core/types';
import {
  Wallet,
  Plus,
  Trash2,
  AlertCircle,
  ArrowDownRight,
  ArrowUpRight,
  Tag,
  X,
  Pencil,
  Check,
  Sparkles,
  BarChart2,
  Receipt,
} from 'lucide-react';
import { triggerAiPrompt } from '../ai';
import { CustomSelect } from '../../components/CustomSelect';
import { PublicTransparencyView } from './components/PublicTransparencyView';
import { NetWorthStatStrip } from './components/NetWorthStatStrip';
import {
  CURRENCY_OPTIONS,
  formatDateInZone,
  nowInZone,
  useSettings,
  zonedInputToUtcIso,
} from '../../core/settings';

export type Direction = 'expense' | 'income';

export interface Transaction {
  id: number;
  room_id: string;
  owner_id: string;
  sender_name: string;
  direction: string;
  /** Minor units — always positive; `direction` carries the sign. */
  amount_minor: number;
  currency: string;
  currency_exponent: number;
  category_id?: number | null;
  category_name?: string | null;
  note?: string | null;
  occurred_at: string;
  created_at?: string | null;
  source_kind?: string | null;
  source_id?: number | null;
}

export interface Category {
  id: number;
  owner_id: string;
  name: string;
  created_at?: string | null;
}

interface LedgerResponse {
  items: Transaction[];
  page: number;
  limit: number;
  total: number;
}

interface CategoryTotal {
  category: string | null;
  expense_minor: number;
  income_minor: number;
}

interface CurrencySummary {
  currency: string;
  currency_exponent: number;
  expense_minor: number;
  income_minor: number;
  net_minor: number;
  categories: CategoryTotal[];
}

/**
 * The picker options for a given value.
 *
 * The API accepts any ISO 4217 code — a Matrix `!expense add 500 JPY ramen`
 * writes one this list does not offer — so an unlisted currency is appended
 * rather than dropped. Without this the select would fall back to its
 * placeholder and saving would blank a currency the user never touched.
 */
export function currencyOptionsFor(current: string) {
  if (!current || CURRENCY_OPTIONS.some((o) => o.value === current)) {
    return CURRENCY_OPTIONS;
  }
  return [...CURRENCY_OPTIONS, { value: current, label: current }];
}

/**
 * Renders minor units as a decimal string.
 *
 * Integer maths only — dividing by 100 into a float is exactly the rounding
 * this whole design avoids on the server.
 */
export function formatMinor(minor: number, exponent: number): string {
  const negative = minor < 0;
  const digits = Math.abs(minor).toString().padStart(exponent + 1, '0');
  const text =
    exponent === 0
      ? digits
      : `${digits.slice(0, digits.length - exponent)}.${digits.slice(digits.length - exponent)}`;
  return negative ? `-${text}` : text;
}

/**
 * The half-open span of the current month, as UTC instants.
 *
 * The month boundary is resolved in the user's chosen zone: someone in
 * Asia/Shanghai is already into the new month while UTC still says the old one,
 * and a summary headed "this month" must agree with the dates listed under it.
 */
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

interface LedgerEditRowProps {
  tx: Transaction;
  categories: Category[];
  saving: boolean;
  onCancel: () => void;
  onSave: (body: Record<string, unknown>) => Promise<void>;
}

const LedgerEditRow: React.FC<LedgerEditRowProps> = ({
  tx,
  categories,
  saving,
  onCancel,
  onSave,
}) => {
  const [direction, setDirection] = useState<Direction>(tx.direction as Direction);
  const [amount, setAmount] = useState(formatMinor(tx.amount_minor, tx.currency_exponent));
  const [currency, setCurrency] = useState(tx.currency);
  const [note, setNote] = useState(tx.note ?? '');
  const [categoryId, setCategoryId] = useState(tx.category_id ? String(tx.category_id) : '');
  const [error, setError] = useState<string | null>(null);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!amount.trim()) {
      setError('Enter an amount.');
      return;
    }
    setError(null);
    try {
      await onSave({
        direction,
        amount: amount.trim(),
        currency: currency.trim().toUpperCase(),
        category_id: categoryId ? Number(categoryId) : null,
        note: note.trim() || null,
        occurred_at: tx.occurred_at,
      });
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save changes.');
    }
  };

  return (
    <form
      onSubmit={submit}
      data-testid="ledger-row-edit"
      className="py-3 px-2 space-y-3 bg-neutral-100/60 dark:bg-neutral-900/30 rounded"
    >
      <div className="grid grid-cols-1 sm:grid-cols-4 gap-3">
        <label className="flex flex-col gap-1">
          <span className="text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans">Direction</span>
          <CustomSelect
            value={direction}
            onChange={(v) => setDirection(v as Direction)}
            mono
            options={[
              { value: 'expense', label: 'Expense' },
              { value: 'income', label: 'Income' },
            ]}
          />
        </label>

        <label className="flex flex-col gap-1">
          <span className="text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans">Amount</span>
          <input
            type="text"
            inputMode="decimal"
            value={amount}
            onChange={(e) => setAmount(e.target.value)}
            className="h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-mono"
          />
        </label>

        <label className="flex flex-col gap-1">
          <span className="text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans">Currency</span>
          <CustomSelect
            value={currency}
            onChange={setCurrency}
            mono
            options={currencyOptionsFor(currency)}
          />
        </label>

        <label className="flex flex-col gap-1">
          <span className="text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans">Category</span>
          <CustomSelect
            value={categoryId}
            onChange={setCategoryId}
            mono
            options={[
              { value: '', label: 'Uncategorised' },
              ...categories.map((c) => ({ value: String(c.id), label: c.name })),
            ]}
          />
        </label>
      </div>

      <input
        type="text"
        value={note}
        placeholder="What was it for?"
        onChange={(e) => setNote(e.target.value)}
        className="w-full h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-sans"
      />

      {error && (
        <div className="flex items-center gap-2 p-2 rounded bg-red-100 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 text-red-800 dark:text-red-300 text-xs font-mono">
          <AlertCircle className="w-4 h-4 shrink-0" />
          <span>{error}</span>
        </div>
      )}

      <div className="flex items-center gap-2">
        <button
          type="submit"
          disabled={saving}
          className="h-9 inline-flex items-center gap-2 px-3 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50 cursor-pointer"
        >
          <Check className="w-3.5 h-3.5" />
          {saving ? 'Saving…' : 'Save'}
        </button>
        <button
          type="button"
          onClick={onCancel}
          className="h-9 inline-flex items-center gap-2 px-3 rounded-md border border-neutral-300 dark:border-neutral-700 text-neutral-600 dark:text-neutral-300 text-xs font-medium hover:bg-neutral-200 dark:hover:bg-neutral-800 transition cursor-pointer"
        >
          <X className="w-3.5 h-3.5" />
          Cancel
        </button>
      </div>
    </form>
  );
};

export const FinanceMainView: React.FC<{ autoFocus?: boolean }> = ({ autoFocus }) => {
  const queryClient = useQueryClient();
  const { timezone, currency: defaultCurrency } = useSettings();
  const amountInputRef = useRef<HTMLInputElement>(null);
  const [direction, setDirection] = useState<Direction>('expense');
  const [amount, setAmount] = useState('');
  const [currency, setCurrency] = useState(defaultCurrency);
  const [note, setNote] = useState('');
  const [categoryId, setCategoryId] = useState('');
  const [filter, setFilter] = useState<'all' | Direction>('all');
  const [editingId, setEditingId] = useState<number | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<'transparency' | 'ledger'>('ledger');

  // The first render happens before the settings query resolves, and the
  // preference can change while this view is mounted; either way the form should
  // show what the user actually chose.
  useEffect(() => {
    setCurrency(defaultCurrency);
  }, [defaultCurrency]);

  useEffect(() => {
    if (autoFocus) amountInputRef.current?.focus();
  }, [autoFocus]);

  const [showCategories, setShowCategories] = useState(false);
  const [newCategory, setNewCategory] = useState('');
  const [categoryError, setCategoryError] = useState<string | null>(null);

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
  };

  const createMutation = useMutation({
    mutationFn: (body: Record<string, unknown>) =>
      fetchApi<Transaction>('/finance/transactions', {
        method: 'POST',
        body: JSON.stringify(body),
      }),
    onSuccess: () => {
      setAmount('');
      setNote('');
      setErrorMsg(null);
      invalidate();
    },
    onError: (err: Error) => setErrorMsg(err.message),
  });

  const deleteMutation = useMutation({
    mutationFn: (id: number) => fetchApi(`/finance/transactions/${id}`, { method: 'DELETE' }),
    onSuccess: invalidate,
  });

  // The API replaces the whole entry, so the edit form always states every
  // field rather than sending a partial patch.
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
      setNewCategory('');
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

  const handleCreate = (e: React.FormEvent) => {
    e.preventDefault();
    if (!amount.trim()) {
      setErrorMsg('Enter an amount.');
      return;
    }
    createMutation.mutate({
      direction,
      // Sent as a string: a JSON number would be a double by the time it
      // reaches the server, and 15.50 does not survive that exactly.
      amount: amount.trim(),
      currency: currency.trim().toUpperCase(),
      category_id: categoryId ? Number(categoryId) : null,
      note: note.trim() || null,
    });
  };

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
        <PublicTransparencyView currency={currency} />
      ) : (
        <>
          {/* Unified cross-currency net worth header widget */}
          <NetWorthStatStrip />

          {/* Per-currency summary — currencies are never added together. */}
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
      <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4">
        <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3">
          <div className="flex items-center gap-2">
            <Wallet className="w-4 h-4 text-neutral-600 dark:text-neutral-300" />
            <h3 className="text-sm font-medium text-neutral-900 dark:text-neutral-200">
              Record Entry
            </h3>
          </div>
          <button
            type="button"
            onClick={() => setShowCategories((open) => !open)}
            className="inline-flex items-center gap-1.5 text-xs font-mono text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-100 transition"
          >
            <Tag className="w-3.5 h-3.5" />
            Categories
          </button>
        </div>

        {errorMsg && (
          <div className="flex items-center gap-2 p-3 rounded bg-red-100 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 text-red-800 dark:text-red-300 text-xs font-mono">
            <AlertCircle className="w-4 h-4 shrink-0" />
            <span>{errorMsg}</span>
          </div>
        )}

        {showCategories && (
          <div
            data-testid="category-manager"
            className="p-4 rounded-lg bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 space-y-3"
          >
            <div className="flex gap-2">
              <input
                type="text"
                placeholder="New category name"
                value={newCategory}
                onChange={(e) => setNewCategory(e.target.value)}
                className="flex-1 h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-sans"
              />
              <button
                type="button"
                onClick={() => newCategory.trim() && addCategoryMutation.mutate(newCategory.trim())}
                className="h-9 px-3 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition cursor-pointer"
              >
                Add
              </button>
            </div>

            {categoryError && (
              <p className="text-xs font-mono text-red-700 dark:text-red-400">{categoryError}</p>
            )}

            <div className="flex flex-wrap gap-2">
              {(categories ?? []).length === 0 ? (
                <span className="text-xs font-mono text-neutral-500">No categories yet.</span>
              ) : (
                (categories ?? []).map((cat) => (
                  <span
                    key={cat.id}
                    className="inline-flex items-center gap-1.5 px-2 py-1 rounded bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-xs font-mono"
                  >
                    {cat.name}
                    <button
                      type="button"
                      title={`Delete category ${cat.name}`}
                      onClick={() => deleteCategoryMutation.mutate(cat.id)}
                      className="text-neutral-400 hover:text-rose-600 dark:hover:text-rose-400 transition"
                    >
                      <X className="w-3 h-3" />
                    </button>
                  </span>
                ))
              )}
            </div>
            <p className="text-[11px] font-mono text-neutral-500">
              Deleting a category keeps its entries and leaves them uncategorised.
            </p>
          </div>
        )}

        <form onSubmit={handleCreate} className="space-y-3">
          <div className="grid grid-cols-1 sm:grid-cols-4 gap-3">
            <label className="flex flex-col gap-1">
              <span className="text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans">Direction</span>
              <CustomSelect
                value={direction}
                onChange={(v) => setDirection(v as Direction)}
                mono
                options={[
                  { value: 'expense', label: 'Expense' },
                  { value: 'income', label: 'Income' },
                ]}
              />
            </label>

            <label className="flex flex-col gap-1">
              <span className="text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans">Amount</span>
              <input
                ref={amountInputRef}
                type="text"
                inputMode="decimal"
                placeholder="15.50"
                value={amount}
                onChange={(e) => setAmount(e.target.value)}
                className="h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-mono"
              />
            </label>

            <label className="flex flex-col gap-1">
              <span className="text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans">Currency</span>
              <CustomSelect
                value={currency}
                onChange={setCurrency}
                mono
                options={currencyOptionsFor(currency)}
              />
            </label>

            <label className="flex flex-col gap-1">
              <span className="text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans">Category</span>
              <CustomSelect
                value={categoryId}
                onChange={setCategoryId}
                mono
                options={[
                  { value: '', label: 'Uncategorised' },
                  ...(categories ?? []).map((c) => ({ value: String(c.id), label: c.name })),
                ]}
              />
            </label>
          </div>

          <div className="grid grid-cols-1 sm:grid-cols-4 gap-3">
            <input
              type="text"
              placeholder="What was it for?"
              value={note}
              onChange={(e) => setNote(e.target.value)}
              className="sm:col-span-3 h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 focus:outline-none focus:border-neutral-500 font-sans"
            />
            <button
              type="submit"
              disabled={createMutation.isPending}
              className="h-9 inline-flex items-center justify-center gap-2 px-4 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50 cursor-pointer"
            >
              <Plus className="w-4 h-4" />
              <span>{createMutation.isPending ? 'Saving...' : 'Record'}</span>
            </button>
          </div>
        </form>
      </div>

      {/* Ledger */}
      <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4">
        <div className="flex flex-wrap items-center justify-between gap-2 border-b border-neutral-200 dark:border-neutral-800 pb-3">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-xs font-medium text-neutral-500 dark:text-neutral-400 whitespace-nowrap">Filter:</span>
            <div className="flex items-center bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded p-0.5 text-xs font-mono">
              {(['all', 'expense', 'income'] as const).map((tab) => (
                <button
                  key={tab}
                  onClick={() => setFilter(tab)}
                  className={`px-2.5 py-1 rounded capitalize transition whitespace-nowrap ${
                    filter === tab
                      ? 'bg-neutral-200 dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100 font-medium'
                      : 'text-neutral-500 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200'
                  }`}
                >
                  {tab}
                </button>
              ))}
            </div>
            <button
              type="button"
              onClick={() =>
                triggerAiPrompt(
                  'Analyze my recent transactions and cashflow. Give me actionable insights on my spending habits and optimization opportunities.'
                )
              }
              className="inline-flex items-center gap-1.5 px-2.5 py-1 text-xs font-sans font-medium text-neutral-700 dark:text-neutral-300 hover:text-neutral-900 dark:hover:text-neutral-100 bg-white dark:bg-neutral-900 hover:bg-neutral-100 dark:hover:bg-neutral-800 border border-neutral-200 dark:border-neutral-800 rounded transition-colors shadow-2xs cursor-pointer"
              title="Ask AI for spending insights"
            >
              <Sparkles className="w-3.5 h-3.5 text-neutral-500 dark:text-neutral-400" />
              <span>AI Insights</span>
            </button>
          </div>
          <span className="text-xs font-mono text-neutral-500 shrink-0">
            {items.length} {items.length === 1 ? 'item' : 'items'}
          </span>
        </div>

        {isLoading ? (
          <div className="py-8 text-center text-xs font-mono text-neutral-500">Loading ledger...</div>
        ) : items.length === 0 ? (
          <div className="py-8 text-center text-xs font-mono text-neutral-500 border border-dashed border-neutral-300 dark:border-neutral-800 rounded">
            Nothing recorded yet.
          </div>
        ) : (
          <div className="divide-y divide-neutral-200 dark:divide-neutral-850">
            {items.map((tx) => {
              const isExpense = tx.direction === 'expense';

              if (editingId === tx.id) {
                return (
                  <LedgerEditRow
                    key={tx.id}
                    tx={tx}
                    categories={categories ?? []}
                    saving={editMutation.isPending}
                    onCancel={() => setEditingId(null)}
                    onSave={async (body) => {
                      await editMutation.mutateAsync({ id: tx.id, ...body });
                      setEditingId(null);
                    }}
                  />
                );
              }

              return (
                <div
                  key={tx.id}
                  data-testid="ledger-row"
                  className="py-3 flex items-center justify-between gap-4 px-2 rounded transition hover:bg-neutral-100/60 dark:hover:bg-neutral-900/50"
                >
                  <div className="flex items-center gap-3 min-w-0">
                    {isExpense ? (
                      <ArrowDownRight className="w-5 h-5 text-rose-500 shrink-0" />
                    ) : (
                      <ArrowUpRight className="w-5 h-5 text-emerald-500 shrink-0" />
                    )}
                    <div className="space-y-0.5 truncate">
                      <p className="text-sm font-medium text-neutral-900 dark:text-neutral-100">
                        {tx.note || (isExpense ? 'Expense' : 'Income')}
                      </p>
                      <div className="flex items-center gap-2 flex-wrap text-xs font-mono text-neutral-500">
                        <span>{formatDateInZone(tx.occurred_at, timezone)}</span>
                        <span>{tx.category_name ?? 'Uncategorised'}</span>
                        {tx.source_kind === 'recurring_bill' && (
                          <span
                            data-testid="badge-source-bill"
                            className="px-1.5 py-0.5 rounded text-[10px] font-medium font-mono bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400 border border-neutral-200 dark:border-neutral-700"
                          >
                            Bill
                          </span>
                        )}
                        {tx.source_kind === 'portfolio_cash' && (
                          <span
                            data-testid="badge-source-portfolio"
                            className="px-1.5 py-0.5 rounded text-[10px] font-medium font-mono bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400 border border-neutral-200 dark:border-neutral-700"
                          >
                            Portfolio
                          </span>
                        )}
                      </div>
                    </div>
                  </div>

                  <div className="flex items-center gap-3 shrink-0">
                    <span
                      className={`font-mono tabular-nums text-sm font-medium ${
                        isExpense
                          ? 'text-rose-600 dark:text-rose-400'
                          : 'text-emerald-600 dark:text-emerald-400'
                      }`}
                    >
                      {isExpense ? '−' : '+'}
                      {formatMinor(tx.amount_minor, tx.currency_exponent)} {tx.currency}
                    </span>
                    <button
                      onClick={() => setEditingId(tx.id)}
                      title="Edit entry"
                      className="p-1.5 rounded text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-200 dark:hover:bg-neutral-800 transition cursor-pointer"
                    >
                      <Pencil className="w-4 h-4" />
                    </button>
                    <button
                      onClick={() => deleteMutation.mutate(tx.id)}
                      title="Delete entry"
                      className="p-1.5 rounded text-neutral-400 hover:text-rose-600 dark:hover:text-rose-400 hover:bg-neutral-200 dark:hover:bg-neutral-800 transition cursor-pointer"
                    >
                      <Trash2 className="w-4 h-4" />
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>
        </>
      )}
    </div>
  );
};

export const financeModule: FrontendModule = {
  id: 'finance',
  name: 'Expenses & Income',
  description: 'Track spending and income per currency, with your own categories',
  navTab: { label: 'Finance' },
  MainView: FinanceMainView,
  handleSseEvent: (event, queryClient) => {
    if (
      event.type === 'TransactionCreated' ||
      event.type === 'TransactionUpdated' ||
      event.type === 'TransactionDeleted'
    ) {
      queryClient.invalidateQueries({ queryKey: ['transactions'] });
      queryClient.invalidateQueries({ queryKey: ['finance-summary'] });
      queryClient.invalidateQueries({ queryKey: ['finance-analytics'] });
      queryClient.invalidateQueries({ queryKey: ['net-worth'] });
    }
    if (event.type === 'CategoryCreated' || event.type === 'CategoryDeleted') {
      queryClient.invalidateQueries({ queryKey: ['finance-categories'] });
      queryClient.invalidateQueries({ queryKey: ['finance-analytics'] });
    }
  },
};
