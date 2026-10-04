import React, { useState } from 'react';
import { ArrowDownRight, X, AlertCircle } from 'lucide-react';
import { DateTimePicker } from '../../components/DateTimePicker';
import { nowInZone, zonedInputToUtcIso } from '../../core/settings';
import { Holding } from './types';
import { formatQuantity } from './utils';

export interface SellRowProps {
  holding: Holding;
  saving: boolean;
  onCancel: () => void;
  onSell: (quantity: string, price: string, executedAt?: string) => Promise<void>;
  timezone: string;
}

export const SellRow: React.FC<SellRowProps> = ({ holding, saving, onCancel, onSell, timezone }) => {
  const [quantity, setQuantity] = useState('');
  const [price, setPrice] = useState('');
  const [sellDate, setSellDate] = useState('');
  const [error, setError] = useState<string | null>(null);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!quantity.trim() || !price.trim()) {
      setError('Enter both a quantity and a price.');
      return;
    }
    setError(null);
    try {
      const executedAt = sellDate ? zonedInputToUtcIso(sellDate, timezone) ?? undefined : undefined;
      await onSell(quantity.trim(), price.trim(), executedAt);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to record the sale.');
    }
  };

  return (
    <form
      onSubmit={submit}
      data-testid="sell-row"
      className="py-3 px-2 space-y-3 bg-neutral-100/60 dark:bg-neutral-900/30 rounded"
    >
      <div className="grid grid-cols-1 sm:grid-cols-4 gap-3">
        <div className="flex flex-col gap-1">
          <div className="flex items-center justify-between">
            <span className="text-[10px] uppercase font-mono text-neutral-500">
              Sell quantity (of {formatQuantity(holding.quantity_minor)})
            </span>
            <button
              type="button"
              onClick={() => setQuantity(formatQuantity(holding.quantity_minor))}
              className="text-[10px] font-mono text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-200 underline decoration-dotted cursor-pointer"
              title="Click to fill max available quantity"
            >
              MAX
            </button>
          </div>
          <div className="relative">
            <input
              type="text"
              inputMode="decimal"
              value={quantity}
              onChange={(e) => setQuantity(e.target.value)}
              placeholder="e.g. 5"
              className="w-full pl-3 pr-14 py-2 rounded bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-mono"
            />
            <button
              type="button"
              onClick={() => setQuantity(formatQuantity(holding.quantity_minor))}
              className="absolute right-1.5 top-1/2 -translate-y-1/2 px-2 py-0.5 rounded text-[10px] font-mono font-bold bg-neutral-200/80 dark:bg-neutral-800 text-neutral-700 dark:text-neutral-300 hover:bg-neutral-300 dark:hover:bg-neutral-700 hover:text-neutral-900 dark:hover:text-neutral-100 transition cursor-pointer"
              title={`Fill max quantity: ${formatQuantity(holding.quantity_minor)}`}
            >
              MAX
            </button>
          </div>
        </div>
        <label className="flex flex-col gap-1">
          <span className="text-[10px] uppercase font-mono text-neutral-500">
            Sale price ({holding.currency})
          </span>
          <input
            type="text"
            inputMode="decimal"
            value={price}
            onChange={(e) => setPrice(e.target.value)}
            placeholder="e.g. 180.00"
            className="px-3 py-2 rounded bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-mono"
          />
        </label>
        <label className="flex flex-col gap-1">
          <span className="text-[10px] uppercase font-mono text-neutral-500">
            Sale Date / Time (Optional)
          </span>
          <DateTimePicker
            value={sellDate}
            onChange={setSellDate}
            placeholder="Now (current time)"
            now={nowInZone(timezone)}
          />
        </label>
        <div className="flex items-end gap-2">
          <button
            type="submit"
            disabled={saving}
            className="inline-flex items-center gap-2 px-3 py-2 rounded bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50"
          >
            <ArrowDownRight className="w-3.5 h-3.5" />
            {saving ? 'Selling…' : 'Sell'}
          </button>
          <button
            type="button"
            onClick={onCancel}
            className="inline-flex items-center gap-2 px-3 py-2 rounded border border-neutral-300 dark:border-neutral-700 text-neutral-600 dark:text-neutral-300 text-xs font-medium hover:bg-neutral-200 dark:hover:bg-neutral-800 transition"
          >
            <X className="w-3.5 h-3.5" />
            Cancel
          </button>
        </div>
      </div>
      {error && (
        <div className="flex items-center gap-2 p-2 rounded bg-red-100 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 text-red-800 dark:text-red-300 text-xs font-mono">
          <AlertCircle className="w-4 h-4 shrink-0" />
          <span>{error}</span>
        </div>
      )}
    </form>
  );
};
