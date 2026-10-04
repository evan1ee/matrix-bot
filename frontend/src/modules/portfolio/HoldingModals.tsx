import React from 'react';
import { Trash2, AlertCircle } from 'lucide-react';
import { TickerSearchInput } from './TickerSearchInput';
import { Holding } from './types';

export interface EditSymbolModalProps {
  holding: Holding;
  symbolInput: string;
  onChangeSymbolInput: (val: string) => void;
  error: string | null;
  isSaving: boolean;
  onClose: () => void;
  onSave: () => void;
}

export const EditSymbolModal: React.FC<EditSymbolModalProps> = ({
  holding,
  symbolInput,
  onChangeSymbolInput,
  error,
  isSaving,
  onClose,
  onSave,
}) => {
  return (
    <div className="fixed inset-0 z-50 bg-black/50 flex items-center justify-center p-4">
      <div className="bg-white dark:bg-neutral-900 rounded-lg border border-neutral-200 dark:border-neutral-800 p-5 max-w-sm w-full space-y-4 shadow-xl">
        <div>
          <h3 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
            Edit Ticker Symbol
          </h3>
          <p className="text-xs text-neutral-500 mt-0.5">
            Update the ticker symbol for this position. Position quantity and cost basis will remain unchanged.
          </p>
        </div>

        {error && (
          <div className="flex items-center gap-2 p-2.5 rounded bg-red-50 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 text-red-700 dark:text-red-300 text-xs">
            <AlertCircle className="w-3.5 h-3.5 shrink-0" />
            <span>{error}</span>
          </div>
        )}

        <div className="space-y-1">
          <label className="text-[10px] uppercase font-mono text-neutral-500">
            New Symbol
          </label>
          <TickerSearchInput
            value={symbolInput}
            onChange={onChangeSymbolInput}
            placeholder="e.g. AAPL, BTC..."
            className="px-3 py-2 rounded bg-neutral-50 dark:bg-neutral-800 border border-neutral-300 dark:border-neutral-700 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-mono uppercase"
          />
        </div>

        <div className="flex items-center justify-end gap-2 pt-2 border-t border-neutral-100 dark:border-neutral-800">
          <button
            type="button"
            onClick={onClose}
            className="px-3 py-1.5 rounded text-xs text-neutral-600 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200 cursor-pointer"
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={!symbolInput.trim() || isSaving}
            onClick={onSave}
            className="px-3.5 py-1.5 rounded bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50 cursor-pointer"
          >
            {isSaving ? 'Saving...' : 'Save Symbol'}
          </button>
        </div>
      </div>
    </div>
  );
};

export interface RemoveHoldingModalProps {
  holding: Holding;
  isDeleting: boolean;
  onClose: () => void;
  onConfirm: () => void;
}

export const RemoveHoldingModal: React.FC<RemoveHoldingModalProps> = ({
  holding,
  isDeleting,
  onClose,
  onConfirm,
}) => {
  return (
    <div className="fixed inset-0 z-50 bg-black/50 flex items-center justify-center p-4">
      <div className="bg-white dark:bg-neutral-900 rounded-lg border border-neutral-200 dark:border-neutral-800 p-5 max-w-sm w-full space-y-4 shadow-xl">
        <div className="flex items-start gap-3">
          <div className="p-2 rounded-full bg-rose-100 dark:bg-rose-950/60 text-rose-600 dark:text-rose-400 shrink-0">
            <Trash2 className="w-5 h-5" />
          </div>
          <div className="space-y-1">
            <h3 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
              Remove {holding.symbol}?
            </h3>
            <p className="text-xs text-neutral-600 dark:text-neutral-400 leading-relaxed">
              This will delete this position from your active portfolio. This action is for administrative corrections and <span className="font-semibold text-neutral-800 dark:text-neutral-200">will not credit your cash balance</span>.
            </p>
            <p className="text-[11px] text-neutral-500 dark:text-neutral-400 pt-1">
              To sell your shares and receive cash proceeds, click <span className="font-medium text-neutral-700 dark:text-neutral-300">Sell (↘)</span> instead.
            </p>
          </div>
        </div>

        <div className="flex items-center justify-end gap-2 pt-2 border-t border-neutral-100 dark:border-neutral-800">
          <button
            type="button"
            onClick={onClose}
            className="px-3 py-1.5 rounded text-xs text-neutral-600 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200 cursor-pointer"
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={isDeleting}
            onClick={onConfirm}
            className="px-3.5 py-1.5 rounded bg-rose-600 hover:bg-rose-700 text-white text-xs font-medium transition disabled:opacity-50 cursor-pointer"
          >
            {isDeleting ? 'Removing...' : 'Remove Position'}
          </button>
        </div>
      </div>
    </div>
  );
};
