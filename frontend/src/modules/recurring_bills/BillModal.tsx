import React, { useState, useEffect } from 'react';
import { AlertCircle, RefreshCcw, X } from 'lucide-react';
import { CustomSelect } from '../../components/CustomSelect';
import { DateTimePicker } from '../../components/DateTimePicker';
import { CURRENCY_OPTIONS, nowInZone, useSettings } from '../../core/settings';
import { CreateBillPayload, RecurringBill, UpdateBillPayload } from './types';

interface BillModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSubmit: (payload: CreateBillPayload | UpdateBillPayload) => void;
  initialData?: RecurringBill | null;
  isSubmitting?: boolean;
  errorMsg?: string | null;
}

const CATEGORY_OPTIONS = [
  { value: 'subscription', label: 'Subscription' },
  { value: 'fixed_bill', label: 'Fixed Bill' },
  { value: 'installment', label: 'Installment' },
  { value: 'recurring_transfer', label: 'Recurring Transfer' },
];

const FREQUENCY_OPTIONS = [
  { value: 'monthly', label: 'Monthly' },
  { value: 'weekly', label: 'Weekly' },
  { value: 'quarterly', label: 'Quarterly' },
  { value: 'yearly', label: 'Yearly' },
  { value: 'daily', label: 'Daily' },
];

export const BillModal: React.FC<BillModalProps> = ({
  isOpen,
  onClose,
  onSubmit,
  initialData,
  isSubmitting = false,
  errorMsg,
}) => {
  const settings = useSettings();
  const defaultCurrency = settings?.currency || 'USD';
  const timezone = settings?.timezone || 'UTC';

  const [name, setName] = useState('');
  const [amount, setAmount] = useState('');
  const [currency, setCurrency] = useState(defaultCurrency);
  const [category, setCategory] = useState('subscription');
  const [frequency, setFrequency] = useState('monthly');
  const [nextDueAt, setNextDueAt] = useState('');
  const [totalInstallments, setTotalInstallments] = useState('');
  const [autoPay, setAutoPay] = useState(false);
  const [notes, setNotes] = useState('');

  useEffect(() => {
    if (initialData) {
      setName(initialData.name);
      setAmount((initialData.amount_minor / 100).toFixed(2));
      setCurrency(initialData.currency);
      setCategory(initialData.category);
      setFrequency(initialData.frequency);
      
      // Parse ISO timestamp to YYYY-MM-DDTHH:mm format for DateTimePicker
      const rawDate = initialData.next_due_at;
      const formatted = rawDate.includes('T')
        ? rawDate.slice(0, 16)
        : rawDate.replace(' ', 'T').slice(0, 16);
      setNextDueAt(formatted);
      
      setTotalInstallments(initialData.total_installments ? String(initialData.total_installments) : '');
      setAutoPay(initialData.auto_pay);
      setNotes(initialData.notes || '');
    } else {
      setName('');
      setAmount('');
      setCurrency(defaultCurrency);
      setCategory('subscription');
      setFrequency('monthly');
      
      // Default to today at 09:00
      const now = nowInZone(timezone);
      const pad = (n: number) => String(n).padStart(2, '0');
      const todayStr = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}T09:00`;
      setNextDueAt(todayStr);

      setTotalInstallments('');
      setAutoPay(false);
      setNotes('');
    }
  }, [initialData, isOpen, defaultCurrency, timezone]);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && isOpen) {
        onClose();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const numAmount = parseFloat(amount);
    if (!name.trim() || isNaN(numAmount) || numAmount <= 0) return;

    const amountMinor = Math.round(numAmount * 100);
    // Format timestamp for backend (YYYY-MM-DD HH:MM:SS)
    let formattedNextDue = `${new Date().toISOString().slice(0, 10)} 09:00:00`;
    if (nextDueAt) {
      const clean = nextDueAt.replace('T', ' ');
      formattedNextDue = clean.length === 16 ? `${clean}:00` : clean.slice(0, 19);
    }

    if (initialData) {
      const payload: UpdateBillPayload = {
        name,
        amount_minor: amountMinor,
        currency,
        category,
        frequency,
        next_due_at: formattedNextDue,
        total_installments: totalInstallments ? parseInt(totalInstallments, 10) : undefined,
        auto_pay: autoPay,
        notes: notes || undefined,
      };
      onSubmit(payload);
    } else {
      const payload: CreateBillPayload = {
        name,
        amount_minor: amountMinor,
        currency,
        category,
        frequency,
        next_due_at: formattedNextDue,
        total_installments: totalInstallments ? parseInt(totalInstallments, 10) : undefined,
        auto_pay: autoPay,
        notes: notes || undefined,
      };
      onSubmit(payload);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 bg-black/60 dark:bg-black/80 backdrop-blur-sm flex items-center justify-center p-4"
      role="dialog"
      aria-modal="true"
    >
      <div className="bg-white dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 rounded-lg max-w-md w-full p-6 space-y-4 shadow-2xl">
        <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3">
          <div className="flex items-center gap-2">
            <RefreshCcw className="w-4 h-4 text-neutral-700 dark:text-neutral-300" />
            <h3 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
              {initialData ? 'Edit Recurring Bill' : 'New Recurring Bill'}
            </h3>
          </div>
          <button
            onClick={onClose}
            className="text-neutral-400 hover:text-neutral-900 dark:hover:text-white transition"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        <form onSubmit={handleSubmit} className="space-y-4">
          {errorMsg && (
            <div className="flex items-center gap-2 p-3 rounded bg-red-100 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 text-red-800 dark:text-red-300 text-xs font-mono">
              <AlertCircle className="w-4 h-4 text-red-500 dark:text-red-400 shrink-0" />
              <span>{errorMsg}</span>
            </div>
          )}
          <div className="space-y-1">
            <label className="text-[10px] uppercase font-mono text-neutral-500">
              Bill / Subscription Name
            </label>
            <input
              type="text"
              required
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Netflix, Rent, Car Loan"
              className="w-full px-3 py-1.5 text-xs bg-transparent border border-neutral-300 dark:border-neutral-700 rounded focus:outline-none focus:ring-1 focus:ring-neutral-400 dark:focus:ring-neutral-500 text-neutral-900 dark:text-neutral-100"
            />
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1">
              <label className="text-[10px] uppercase font-mono text-neutral-500">
                Category
              </label>
              <CustomSelect
                value={category}
                onChange={setCategory}
                options={CATEGORY_OPTIONS}
                mono
                className="w-full"
              />
            </div>
            <div className="space-y-1">
              <label className="text-[10px] uppercase font-mono text-neutral-500">
                Next Due Date
              </label>
              <DateTimePicker
                value={nextDueAt}
                onChange={setNextDueAt}
                placeholder="Select date"
                now={nowInZone(timezone)}
                showTime={true}
              />
            </div>
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1">
              <label className="text-[10px] uppercase font-mono text-neutral-500">
                Frequency
              </label>
              <CustomSelect
                value={frequency}
                onChange={setFrequency}
                options={FREQUENCY_OPTIONS}
                mono
                className="w-full"
              />
            </div>
            <div className="space-y-1">
              <label className="text-[10px] uppercase font-mono text-neutral-500">
                Amount
              </label>
              <input
                type="number"
                step="0.01"
                min="0.01"
                required
                value={amount}
                onChange={(e) => setAmount(e.target.value)}
                placeholder="0.00"
                className="w-full px-3 py-1.5 text-xs bg-transparent border border-neutral-300 dark:border-neutral-700 rounded focus:outline-none focus:ring-1 focus:ring-neutral-400 dark:focus:ring-neutral-500 text-neutral-900 dark:text-neutral-100 font-mono"
              />
            </div>
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1">
              <label className="text-[10px] uppercase font-mono text-neutral-500">
                Currency
              </label>
              <CustomSelect
                value={currency}
                onChange={setCurrency}
                options={CURRENCY_OPTIONS}
                mono
                className="w-full"
              />
            </div>

            {category === 'installment' && (
              <div className="space-y-1">
                <label className="text-[10px] uppercase font-mono text-neutral-500">
                  Total Installments
                </label>
                <input
                  type="number"
                  min="1"
                  value={totalInstallments}
                  onChange={(e) => setTotalInstallments(e.target.value)}
                  placeholder="e.g. 12"
                  className="w-full px-3 py-1.5 text-xs bg-transparent border border-neutral-300 dark:border-neutral-700 rounded focus:outline-none focus:ring-1 focus:ring-neutral-400 dark:focus:ring-neutral-500 text-neutral-900 dark:text-neutral-100 font-mono"
                />
              </div>
            )}
          </div>

          <div className="flex items-center gap-2 pt-1">
            <input
              type="checkbox"
              id="autoPay"
              checked={autoPay}
              onChange={(e) => setAutoPay(e.target.checked)}
              className="rounded border-neutral-300 dark:border-neutral-700 text-neutral-900 dark:text-neutral-100 focus:ring-neutral-400"
            />
            <label htmlFor="autoPay" className="text-xs text-neutral-700 dark:text-neutral-300 cursor-pointer">
              Enable Auto-pay (Auto debit)
            </label>
          </div>

          <div className="space-y-1">
            <label className="text-[10px] uppercase font-mono text-neutral-500">
              Notes (Optional)
            </label>
            <textarea
              rows={2}
              value={notes}
              onChange={(e) => setNotes(e.target.value)}
              placeholder="e.g. Card ending in 1234, contract ID"
              className="w-full px-3 py-1.5 text-xs bg-transparent border border-neutral-300 dark:border-neutral-700 rounded focus:outline-none focus:ring-1 focus:ring-neutral-400 dark:focus:ring-neutral-500 text-neutral-900 dark:text-neutral-100"
            />
          </div>

          <div className="pt-3 flex items-center justify-end gap-3 border-t border-neutral-200 dark:border-neutral-800">
            <button
              type="button"
              onClick={onClose}
              className="text-xs font-mono text-neutral-600 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200 transition"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={isSubmitting}
              className="px-4 py-2 rounded bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-white transition disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {isSubmitting ? 'Saving...' : initialData ? 'Save Changes' : 'Create Item'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
