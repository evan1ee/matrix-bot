import React, { useState, useEffect } from 'react';
import { Wallet, AlertCircle, ArrowUpRight, ArrowDownRight } from 'lucide-react';
import { CustomSelect } from '../../components/CustomSelect';
import { DateTimePicker } from '../../components/DateTimePicker';
import { nowInZone, zonedInputToUtcIso } from '../../core/settings';
import { CashBalance } from './types';
import { formatMinor, currencyOptionsFor } from './utils';

export interface CashCardProps {
  cashBalances?: CashBalance[];
  defaultCurrency: string;
  timezone: string;
  isPending: boolean;
  onDeposit: (body: { amount: string; currency: string; executed_at?: string }) => void;
  onWithdraw: (body: { amount: string; currency: string; executed_at?: string }) => void;
  error: string | null;
}

export const CashCard: React.FC<CashCardProps> = ({
  cashBalances,
  defaultCurrency,
  timezone,
  isPending,
  onDeposit,
  onWithdraw,
  error,
}) => {
  const [cashAmount, setCashAmount] = useState('');
  const [cashCurrency, setCashCurrency] = useState(defaultCurrency);
  const [cashDate, setCashDate] = useState('');
  const [localError, setLocalError] = useState<string | null>(null);

  useEffect(() => {
    setCashCurrency(defaultCurrency);
  }, [defaultCurrency]);

  const submitCash = (action: 'deposit' | 'withdraw') => {
    if (!cashAmount.trim()) {
      setLocalError('Enter an amount.');
      return;
    }
    setLocalError(null);
    const executed_at = cashDate ? zonedInputToUtcIso(cashDate, timezone) ?? undefined : undefined;
    const body = {
      amount: cashAmount.trim(),
      currency: cashCurrency.trim().toUpperCase(),
      executed_at,
    };
    if (action === 'deposit') {
      onDeposit(body);
    } else {
      onWithdraw(body);
    }
    setCashAmount('');
    setCashDate('');
  };

  const displayError = error || localError;

  return (
    <div
      data-testid="cash-panel"
      className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4"
    >
      <div className="flex items-center gap-2 border-b border-neutral-200 dark:border-neutral-800 pb-3">
        <Wallet className="w-4 h-4 text-neutral-600 dark:text-neutral-300" />
        <h3 className="text-sm font-medium text-neutral-900 dark:text-neutral-200">Cash</h3>
      </div>

      {displayError && (
        <div className="flex items-center gap-2 p-3 rounded bg-red-100 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 text-red-800 dark:text-red-300 text-xs font-mono">
          <AlertCircle className="w-4 h-4 shrink-0" />
          <span>{displayError}</span>
        </div>
      )}

      {(cashBalances ?? []).length > 0 && (
        <div className="flex flex-wrap gap-2" data-testid="cash-balances">
          {(cashBalances ?? []).map((c) => (
            <span
              key={c.currency}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-sm font-mono text-neutral-900 dark:text-neutral-100"
            >
              {formatMinor(c.balance_minor, c.currency_exponent)} {c.currency}
            </span>
          ))}
        </div>
      )}

      <div className="grid grid-cols-1 sm:grid-cols-12 gap-3 items-end">
        <label className="flex flex-col gap-1 sm:col-span-4">
          <span className="text-[10px] uppercase font-mono text-neutral-500">Amount</span>
          <input
            type="text"
            inputMode="decimal"
            placeholder="500.00"
            value={cashAmount}
            onChange={(e) => setCashAmount(e.target.value)}
            className="h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-mono"
          />
        </label>
        <label className="flex flex-col gap-1 sm:col-span-2">
          <span className="text-[10px] uppercase font-mono text-neutral-500">Currency</span>
          <CustomSelect
            value={cashCurrency}
            onChange={setCashCurrency}
            mono
            options={currencyOptionsFor(cashCurrency)}
          />
        </label>
        <label className="flex flex-col gap-1 sm:col-span-3">
          <span className="text-[10px] uppercase font-mono text-neutral-500">Date / Time (Optional)</span>
          <DateTimePicker
            value={cashDate}
            onChange={setCashDate}
            placeholder="Now (current time)"
            now={nowInZone(timezone)}
          />
        </label>
        <div className="flex gap-2 sm:col-span-3">
          <button
            type="button"
            onClick={() => submitCash('deposit')}
            disabled={isPending}
            className="h-9 flex-1 inline-flex items-center justify-center gap-1.5 px-3 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50 cursor-pointer"
          >
            <ArrowUpRight className="w-3.5 h-3.5" />
            Deposit
          </button>
          <button
            type="button"
            onClick={() => submitCash('withdraw')}
            disabled={isPending}
            className="h-9 flex-1 inline-flex items-center justify-center gap-1.5 px-3 rounded-md border border-neutral-300 dark:border-neutral-700 text-neutral-700 dark:text-neutral-300 text-xs font-medium hover:bg-neutral-200 dark:hover:bg-neutral-800 transition disabled:opacity-50 cursor-pointer"
          >
            <ArrowDownRight className="w-3.5 h-3.5" />
            Withdraw
          </button>
        </div>
      </div>
      <p className="text-[11px] font-mono text-neutral-500">
        Buying a position debits cash automatically; selling credits it back.
      </p>
    </div>
  );
};
