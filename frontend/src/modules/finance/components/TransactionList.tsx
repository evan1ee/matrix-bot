import React, { useState } from 'react';
import {
  ArrowDownRight,
  ArrowUpRight,
  Paperclip,
  Pencil,
  Trash2,
  Sparkles,
  AlertCircle,
  Check,
  X,
} from 'lucide-react';
import { triggerAiPrompt } from '../../ai';
import { CustomSelect } from '../../../components/CustomSelect';
import { formatDateInZone } from '../../../core/settings';
import { ReceiptLightboxModal } from './ReceiptLightboxModal';
import { Transaction, Category, Direction, currencyOptionsFor, formatMinor } from '../index';

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

export interface TransactionListProps {
  items: Transaction[];
  isLoading: boolean;
  categories: Category[];
  filter: 'all' | Direction;
  onFilterChange: (filter: 'all' | Direction) => void;
  onEdit: (id: number, body: Record<string, unknown>) => Promise<void>;
  onDelete: (id: number) => void;
  isEditing: boolean;
  timezone: string;
}

export const TransactionList: React.FC<TransactionListProps> = ({
  items,
  isLoading,
  categories,
  filter,
  onFilterChange,
  onEdit,
  onDelete,
  isEditing,
  timezone,
}) => {
  const [editingId, setEditingId] = useState<number | null>(null);
  const [receiptTx, setReceiptTx] = useState<Transaction | null>(null);

  return (
    <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-neutral-200 dark:border-neutral-800 pb-3">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-xs font-medium text-neutral-500 dark:text-neutral-400 whitespace-nowrap">Filter:</span>
          <div className="flex items-center bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded p-0.5 text-xs font-mono">
            {(['all', 'expense', 'income'] as const).map((tab) => (
              <button
                key={tab}
                onClick={() => onFilterChange(tab)}
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
                  categories={categories}
                  saving={isEditing}
                  onCancel={() => setEditingId(null)}
                  onSave={async (body) => {
                    await onEdit(tx.id, body);
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
                  {tx.receipt_storage_key && (
                    <button
                      onClick={() => setReceiptTx(tx)}
                      title="View receipt"
                      aria-label="View receipt"
                      className="p-1.5 rounded text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-200 dark:hover:bg-neutral-800 transition cursor-pointer"
                    >
                      <Paperclip className="w-4 h-4" />
                    </button>
                  )}
                  <button
                    onClick={() => setEditingId(tx.id)}
                    title="Edit entry"
                    className="p-1.5 rounded text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-200 dark:hover:bg-neutral-800 transition cursor-pointer"
                  >
                    <Pencil className="w-4 h-4" />
                  </button>
                  <button
                    onClick={() => onDelete(tx.id)}
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
      {receiptTx && (
        <ReceiptLightboxModal
          transactionId={receiptTx.id}
          title={receiptTx.note || 'Receipt'}
          onClose={() => setReceiptTx(null)}
        />
      )}
    </div>
  );
};
