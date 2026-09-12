import React, { useState, useEffect } from 'react';
import { useQuery } from '@tanstack/react-query';
import { AlertCircle, RefreshCcw, X } from 'lucide-react';
import { fetchApi } from '../../core/apiClient';
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

/** Mirrors the todo module's lead options, so both features offer one vocabulary. */
const LEAD_OPTIONS = [
  { value: '', label: 'No reminder' },
  { value: '60', label: '1 hour before' },
  { value: '1440', label: '1 day before' },
  { value: '4320', label: '3 days before' },
  { value: '10080', label: '1 week before' },
];

interface JoinedRoomInfo {
  room_id: string;
  name: string;
}

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
  const [leadMinutes, setLeadMinutes] = useState('');
  const [remindRoomId, setRemindRoomId] = useState('');

  // Rooms the bot and this account share. A bill created here carries the `web`
  // sentinel, so a reminder has nowhere to go until one is picked.
  const { data: rooms } = useQuery<JoinedRoomInfo[]>({
    queryKey: ['rooms'],
    queryFn: () => fetchApi<JoinedRoomInfo[]>('/rooms'),
  });

  const wantsReminder = leadMinutes !== '';
  const noRoomsAvailable = wantsReminder && (rooms?.length ?? 0) === 0;

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
      setLeadMinutes(
        initialData.remind_before_minutes ? String(initialData.remind_before_minutes) : ''
      );
      setRemindRoomId(initialData.remind_room_id || '');
    } else {
      setName('');
      setAmount('');
      setCurrency(defaultCurrency);
      setCategory('subscription');
      setFrequency('monthly');
      setLeadMinutes('');
      setRemindRoomId('');
      
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
        remind_before_minutes: wantsReminder ? parseInt(leadMinutes, 10) : null,
        remind_room_id: remindRoomId || undefined,
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
        remind_before_minutes: wantsReminder ? parseInt(leadMinutes, 10) : undefined,
        remind_room_id: remindRoomId || undefined,
      };
      onSubmit(payload);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 bg-black/60 dark:bg-black/80 flex items-center justify-center p-4"
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
              className="w-full h-9 px-3 text-xs bg-transparent border border-neutral-300 dark:border-neutral-700 rounded-md focus:outline-none focus:ring-1 focus:ring-neutral-400 dark:focus:ring-neutral-500 text-neutral-900 dark:text-neutral-100"
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
                className="w-full h-9 px-3 text-xs bg-transparent border border-neutral-300 dark:border-neutral-700 rounded-md focus:outline-none focus:ring-1 focus:ring-neutral-400 dark:focus:ring-neutral-500 text-neutral-900 dark:text-neutral-100 font-mono"
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
                  className="w-full h-9 px-3 text-xs bg-transparent border border-neutral-300 dark:border-neutral-700 rounded-md focus:outline-none focus:ring-1 focus:ring-neutral-400 dark:focus:ring-neutral-500 text-neutral-900 dark:text-neutral-100 font-mono"
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

          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1">
              <label className="text-[10px] uppercase font-mono text-neutral-500">
                Remind me
              </label>
              <CustomSelect
                value={leadMinutes}
                onChange={setLeadMinutes}
                mono
                options={LEAD_OPTIONS}
                className="w-full"
              />
            </div>

            {wantsReminder && (
              <div className="space-y-1">
                <label className="text-[10px] uppercase font-mono text-neutral-500">
                  Send to room
                </label>
                <CustomSelect
                  value={remindRoomId}
                  onChange={setRemindRoomId}
                  mono
                  className="w-full"
                  options={[
                    { value: '', label: 'Select a room…' },
                    ...(rooms ?? []).map((r) => ({ value: r.room_id, label: r.name })),
                  ]}
                />
              </div>
            )}
          </div>

          {noRoomsAvailable && (
            <p className="text-[11px] font-mono text-amber-600 dark:text-amber-400">
              A reminder needs a Matrix room the bot is also in. Invite the bot to a room to use this.
            </p>
          )}

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
              className="h-9 px-3 rounded-md text-xs font-mono text-neutral-600 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200 transition"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={isSubmitting}
              className="h-9 px-4 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-white transition disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer"
            >
              {isSubmitting ? 'Saving...' : initialData ? 'Save Changes' : 'Create Item'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
