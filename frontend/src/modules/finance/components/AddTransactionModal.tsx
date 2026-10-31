import React, { useRef, useState, useEffect } from 'react';
import { Wallet, Tag, AlertCircle, Plus, X } from 'lucide-react';
import { CustomSelect } from '../../../components/CustomSelect';
import { ReceiptDropzone } from './ReceiptDropzone';
import { Category, Direction, currencyOptionsFor } from '../index';
import { ReceiptExtraction } from '../types';

export interface AddTransactionModalProps {
  categories: Category[];
  defaultCurrency: string;
  onRecord: (body: Record<string, unknown>) => void;
  isRecording: boolean;
  onAddCategory: (name: string) => void;
  onDeleteCategory: (id: number) => void;
  errorMsg: string | null;
  categoryError: string | null;
  autoFocus?: boolean;
}

export const AddTransactionModal: React.FC<AddTransactionModalProps> = ({
  categories,
  defaultCurrency,
  onRecord,
  isRecording,
  onAddCategory,
  onDeleteCategory,
  errorMsg,
  categoryError,
  autoFocus,
}) => {
  const amountInputRef = useRef<HTMLInputElement>(null);
  const [direction, setDirection] = useState<Direction>('expense');
  const [amount, setAmount] = useState('');
  const [currency, setCurrency] = useState(defaultCurrency);
  const [note, setNote] = useState('');
  const [categoryId, setCategoryId] = useState('');
  const [showCategories, setShowCategories] = useState(false);
  const [newCategory, setNewCategory] = useState('');
  const [localError, setLocalError] = useState<string | null>(null);
  const [receiptKey, setReceiptKey] = useState<string | null>(null);

  useEffect(() => {
    setCurrency(defaultCurrency);
  }, [defaultCurrency]);

  useEffect(() => {
    if (autoFocus) amountInputRef.current?.focus();
  }, [autoFocus]);

  const handleCreate = (e: React.FormEvent) => {
    e.preventDefault();
    if (!amount.trim()) {
      setLocalError('Enter an amount.');
      return;
    }
    setLocalError(null);
    onRecord({
      direction,
      amount: amount.trim(),
      currency: currency.trim().toUpperCase(),
      category_id: categoryId ? Number(categoryId) : null,
      note: note.trim() || null,
      receipt_storage_key: receiptKey,
    });
    setAmount('');
    setNote('');
    setReceiptKey(null);
  };

  const handleReceiptScanned = (extracted: ReceiptExtraction, receiptStorageKey: string) => {
    setReceiptKey(receiptStorageKey);
    if (extracted.amount) {
      setAmount(extracted.amount);
    }
    if (extracted.currency) {
      setCurrency(extracted.currency.toUpperCase());
    }
    if (extracted.merchant) {
      setNote(extracted.merchant);
    }
    if (extracted.category_suggestion && categories) {
      const match = categories.find(
        (c) => c.name.toLowerCase() === extracted.category_suggestion!.toLowerCase()
      );
      if (match) {
        setCategoryId(String(match.id));
      }
    }
    setDirection('expense');
  };

  const displayError = errorMsg || localError;

  return (
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
          className="inline-flex items-center gap-1.5 text-xs font-mono text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-100 transition cursor-pointer"
        >
          <Tag className="w-3.5 h-3.5" />
          Categories
        </button>
      </div>

      {displayError && (
        <div className="flex items-center gap-2 p-3 rounded bg-red-100 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 text-red-800 dark:text-red-300 text-xs font-mono">
          <AlertCircle className="w-4 h-4 shrink-0" />
          <span>{displayError}</span>
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
              onClick={() => {
                if (newCategory.trim()) {
                  onAddCategory(newCategory.trim());
                  setNewCategory('');
                }
              }}
              className="h-9 px-3 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition cursor-pointer"
            >
              Add
            </button>
          </div>

          {categoryError && (
            <p className="text-xs font-mono text-red-700 dark:text-red-400">{categoryError}</p>
          )}

          <div className="flex flex-wrap gap-2">
            {categories.length === 0 ? (
              <span className="text-xs font-mono text-neutral-500">No categories yet.</span>
            ) : (
              categories.map((cat) => (
                <span
                  key={cat.id}
                  className="inline-flex items-center gap-1.5 px-2 py-1 rounded bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-xs font-mono"
                >
                  {cat.name}
                  <button
                    type="button"
                    title={`Delete category ${cat.name}`}
                    onClick={() => onDeleteCategory(cat.id)}
                    className="text-neutral-400 hover:text-rose-600 dark:hover:text-rose-400 transition cursor-pointer"
                  >
                    <X className="w-3.5 h-3.5" />
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

      <ReceiptDropzone onReceiptScanned={handleReceiptScanned} />

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
                ...categories.map((c) => ({ value: String(c.id), label: c.name })),
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
            disabled={isRecording}
            className="h-9 inline-flex items-center justify-center gap-2 px-4 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50 cursor-pointer"
          >
            <Plus className="w-4 h-4" />
            <span>{isRecording ? 'Saving...' : 'Record'}</span>
          </button>
        </div>
      </form>
    </div>
  );
};
