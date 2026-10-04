import React, { useState, useEffect } from 'react';
import { Briefcase, AlertCircle, Plus, BookmarkPlus } from 'lucide-react';
import { CustomSelect } from '../../components/CustomSelect';
import { DateTimePicker } from '../../components/DateTimePicker';
import { TickerSearchInput } from './TickerSearchInput';
import { nowInZone, zonedInputToUtcIso } from '../../core/settings';
import { currencyOptionsFor } from './utils';

export interface BuyCardProps {
  defaultCurrency: string;
  timezone: string;
  isPending: boolean;
  onBuy: (body: Record<string, unknown>) => void;
  onRecord: (body: Record<string, unknown>) => void;
  error: string | null;
}

export const BuyCard: React.FC<BuyCardProps> = ({
  defaultCurrency,
  timezone,
  isPending,
  onBuy,
  onRecord,
  error,
}) => {
  const [symbol, setSymbol] = useState('');
  const [quantity, setQuantity] = useState('');
  const [price, setPrice] = useState('');
  const [currency, setCurrency] = useState(defaultCurrency);
  const [note, setNote] = useState('');
  const [transactionDate, setTransactionDate] = useState('');
  const [localError, setLocalError] = useState<string | null>(null);

  useEffect(() => {
    setCurrency(defaultCurrency);
  }, [defaultCurrency]);

  const resetForm = () => {
    setSymbol('');
    setQuantity('');
    setPrice('');
    setNote('');
    setTransactionDate('');
    setLocalError(null);
  };

  const handleBuy = (e: React.FormEvent) => {
    e.preventDefault();
    if (!symbol.trim() || !quantity.trim() || !price.trim()) {
      setLocalError('Enter a symbol, quantity, and price.');
      return;
    }
    setLocalError(null);
    const executed_at = transactionDate ? zonedInputToUtcIso(transactionDate, timezone) ?? undefined : undefined;
    onBuy({
      symbol: symbol.trim().toUpperCase(),
      quantity: quantity.trim(),
      price: price.trim(),
      currency: currency.trim().toUpperCase(),
      note: note.trim() || null,
      executed_at,
    });
    resetForm();
  };

  const handleRecord = (e: React.MouseEvent) => {
    e.preventDefault();
    if (!symbol.trim() || !quantity.trim() || !price.trim()) {
      setLocalError('Enter a symbol, quantity, and cost price.');
      return;
    }
    setLocalError(null);
    const executed_at = transactionDate ? zonedInputToUtcIso(transactionDate, timezone) ?? undefined : undefined;
    onRecord({
      symbol: symbol.trim().toUpperCase(),
      quantity: quantity.trim(),
      price: price.trim(),
      currency: currency.trim().toUpperCase(),
      note: note.trim() || null,
      executed_at,
    });
    resetForm();
  };

  const displayError = error || localError;

  return (
    <div
      data-testid="buy-panel"
      className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4"
    >
      <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3">
        <div className="flex items-center gap-2">
          <Briefcase className="w-4 h-4 text-neutral-600 dark:text-neutral-300" />
          <h3 className="text-sm font-medium text-neutral-900 dark:text-neutral-200">
            Add Holding
          </h3>
        </div>
        <span className="text-xs font-mono text-neutral-500">Cash / Manual Record</span>
      </div>

      {displayError && (
        <div className="flex items-center gap-2 p-3 rounded bg-red-100 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 text-red-800 dark:text-red-300 text-xs font-mono">
          <AlertCircle className="w-4 h-4 shrink-0" />
          <span>{displayError}</span>
        </div>
      )}

      <form onSubmit={handleBuy} className="space-y-3">
        <div className="grid grid-cols-1 sm:grid-cols-12 gap-3">
          <div className="flex flex-col gap-1 sm:col-span-3">
            <span className="text-[10px] uppercase font-mono text-neutral-500">Symbol</span>
            <TickerSearchInput
              value={symbol}
              onChange={setSymbol}
              placeholder="e.g. AAPL, BTC, VOO"
              className="h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-mono uppercase"
            />
          </div>

          <label className="flex flex-col gap-1 sm:col-span-2">
            <span className="text-[10px] uppercase font-mono text-neutral-500">Quantity</span>
            <input
              type="text"
              inputMode="decimal"
              placeholder="10"
              value={quantity}
              onChange={(e) => setQuantity(e.target.value)}
              className="h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-mono"
            />
          </label>

          <label className="flex flex-col gap-1 sm:col-span-2">
            <span className="text-[10px] uppercase font-mono text-neutral-500">Price</span>
            <input
              type="text"
              inputMode="decimal"
              placeholder="150.00"
              value={price}
              onChange={(e) => setPrice(e.target.value)}
              className="h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-mono"
            />
          </label>

          <label className="flex flex-col gap-1 sm:col-span-2">
            <span className="text-[10px] uppercase font-mono text-neutral-500">Currency</span>
            <CustomSelect
              value={currency}
              onChange={setCurrency}
              mono
              options={currencyOptionsFor(currency)}
            />
          </label>

          <label className="flex flex-col gap-1 sm:col-span-3">
            <span className="text-[10px] uppercase font-mono text-neutral-500">Date / Time (Optional)</span>
            <DateTimePicker
              value={transactionDate}
              onChange={setTransactionDate}
              placeholder="Now (current time)"
              now={nowInZone(timezone)}
            />
          </label>
        </div>

        <div className="grid grid-cols-1 sm:grid-cols-12 gap-3">
          <input
            type="text"
            placeholder="Note (optional)"
            value={note}
            onChange={(e) => setNote(e.target.value)}
            className="sm:col-span-6 h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 focus:outline-none focus:border-neutral-500 font-sans"
          />
          <button
            type="submit"
            disabled={isPending}
            className="sm:col-span-3 h-9 inline-flex items-center justify-center gap-1.5 px-3 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50 cursor-pointer"
            title="Buy position and deduct cost from cash balance"
          >
            <Plus className="w-3.5 h-3.5" />
            <span>{isPending ? 'Buying...' : 'Buy'}</span>
          </button>
          <button
            type="button"
            disabled={isPending}
            onClick={handleRecord}
            className="sm:col-span-3 h-9 inline-flex items-center justify-center gap-1.5 px-3 rounded-md bg-neutral-100 dark:bg-neutral-800 text-neutral-700 dark:text-neutral-200 border border-neutral-200 dark:border-neutral-700 text-xs font-medium hover:bg-neutral-200 dark:hover:bg-neutral-700 transition disabled:opacity-50 cursor-pointer"
            title="Record already-owned stock or crypto without deducting cash balance"
          >
            <BookmarkPlus className="w-3.5 h-3.5" />
            <span>{isPending ? 'Recording...' : 'Record Existing (No Cash)'}</span>
          </button>
        </div>
      </form>
    </div>
  );
};
