import React, { useState } from 'react';
import { Pencil, Tag, X, Check } from 'lucide-react';
import { Holding } from './types';
import { formatMinor } from './utils';

export interface NoteRowProps {
  holding: Holding;
  saving: boolean;
  onCancel: () => void;
  onSave: (note: string) => Promise<void>;
}

export const NoteRow: React.FC<NoteRowProps> = ({ holding, saving, onCancel, onSave }) => {
  const [note, setNote] = useState(holding.note ?? '');
  const [error, setError] = useState<string | null>(null);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    try {
      await onSave(note.trim());
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save the note.');
    }
  };

  return (
    <form onSubmit={submit} className="flex items-center gap-2">
      <input
        type="text"
        autoFocus
        value={note}
        onChange={(e) => setNote(e.target.value)}
        placeholder="Note (leave blank to clear)"
        className="w-56 px-2 py-1 rounded bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-sans"
      />
      <button
        type="submit"
        disabled={saving}
        title="Save note"
        className="p-1.5 rounded text-neutral-600 dark:text-neutral-300 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-200 dark:hover:bg-neutral-900 transition"
      >
        <Check className="w-3.5 h-3.5" />
      </button>
      <button
        type="button"
        onClick={onCancel}
        title="Cancel"
        className="p-1.5 rounded text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-200 dark:hover:bg-neutral-900 transition"
      >
        <X className="w-3.5 h-3.5" />
      </button>
      {error && <span className="text-[11px] font-mono text-rose-600 dark:text-rose-400">{error}</span>}
    </form>
  );
};

export interface PriceRowProps {
  holding: Holding;
  saving: boolean;
  onCancel: () => void;
  onSave: (price: string) => Promise<void>;
}

export const PriceRow: React.FC<PriceRowProps> = ({ holding, saving, onCancel, onSave }) => {
  const [price, setPrice] = useState(
    holding.current_price_minor != null
      ? formatMinor(holding.current_price_minor, holding.currency_exponent)
      : ''
  );
  const [error, setError] = useState<string | null>(null);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!price.trim()) {
      setError('Enter a price.');
      return;
    }
    setError(null);
    try {
      await onSave(price.trim());
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save the price.');
    }
  };

  return (
    <form onSubmit={submit} className="flex items-center gap-2">
      <input
        type="text"
        inputMode="decimal"
        autoFocus
        value={price}
        onChange={(e) => setPrice(e.target.value)}
        placeholder={`Price (${holding.currency})`}
        className="w-28 px-2 py-1 rounded bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-mono"
      />
      <button
        type="submit"
        disabled={saving}
        title="Save price"
        className="p-1.5 rounded text-neutral-600 dark:text-neutral-300 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-200 dark:hover:bg-neutral-900 transition"
      >
        <Check className="w-3.5 h-3.5" />
      </button>
      <button
        type="button"
        onClick={onCancel}
        title="Cancel"
        className="p-1.5 rounded text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-200 dark:hover:bg-neutral-900 transition"
      >
        <X className="w-3.5 h-3.5" />
      </button>
      {error && <span className="text-[11px] font-mono text-rose-600 dark:text-rose-400">{error}</span>}
    </form>
  );
};
