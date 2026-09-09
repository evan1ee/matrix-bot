import React, { useState, useEffect } from 'react';
import { X } from 'lucide-react';
import type { CreateHabitPayload, HabitWithStatus } from './types';

interface HabitModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSave: (payload: CreateHabitPayload, id?: number) => Promise<void>;
  initialData?: HabitWithStatus | null;
}

export const HabitModal: React.FC<HabitModalProps> = ({
  isOpen,
  onClose,
  onSave,
  initialData,
}) => {
  const [name, setName] = useState('');
  const [category, setCategory] = useState('wellness');
  const [frequency, setFrequency] = useState('daily');
  const [targetDays, setTargetDays] = useState(7);
  const [description, setDescription] = useState('');
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (initialData) {
      setName(initialData.name);
      setCategory(initialData.category || 'wellness');
      setFrequency(initialData.frequency || 'daily');
      setTargetDays(initialData.target_days_per_week || 7);
      setDescription(initialData.description || '');
    } else {
      setName('');
      setCategory('wellness');
      setFrequency('daily');
      setTargetDays(7);
      setDescription('');
    }
    setError(null);
  }, [initialData, isOpen]);

  if (!isOpen) return null;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const trimmed = name.trim();
    if (!trimmed) {
      setError('Habit name is required.');
      return;
    }
    if (trimmed.length > 100) {
      setError('Habit name must be 100 characters or fewer.');
      return;
    }

    setIsSubmitting(true);
    setError(null);

    try {
      await onSave(
        {
          name: trimmed,
          category,
          frequency,
          target_days_per_week: targetDays,
          description: description.trim() || undefined,
        },
        initialData?.id
      );
      onClose();
    } catch (err: any) {
      setError(err?.message || 'Failed to save habit');
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs">
      <div
        data-testid="habit-modal"
        className="w-full max-w-md rounded-lg bg-white dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-6 shadow-xl space-y-5"
      >
        {/* Header */}
        <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3">
          <h3 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100 font-sans">
            {initialData ? 'Edit Habit' : 'New Habit'}
          </h3>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close dialog"
            className="p-1 rounded text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-200 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition cursor-pointer"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {error && (
          <div className="p-2.5 rounded text-xs bg-red-500/10 text-red-600 dark:text-red-400 border border-red-500/20 font-sans">
            {error}
          </div>
        )}

        <form onSubmit={handleSubmit} className="space-y-4">
          {/* Name */}
          <div className="space-y-1.5">
            <label className="text-[10px] font-mono uppercase tracking-wider text-neutral-500">
              Habit Name *
            </label>
            <input
              type="text"
              required
              maxLength={100}
              placeholder="e.g., Morning Run, Read 30 mins, Code"
              value={name}
              onChange={(e) => setName(e.target.value)}
              className="w-full h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 focus:outline-none focus:border-neutral-500 font-sans"
            />
          </div>

          {/* Category & Frequency Row */}
          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1.5">
              <label className="text-[10px] font-mono uppercase tracking-wider text-neutral-500">
                Category
              </label>
              <select
                value={category}
                onChange={(e) => setCategory(e.target.value)}
                className="w-full h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-sans"
              >
                <option value="wellness">Wellness</option>
                <option value="fitness">Fitness</option>
                <option value="learning">Learning</option>
                <option value="productivity">Productivity</option>
                <option value="mindfulness">Mindfulness</option>
              </select>
            </div>

            <div className="space-y-1.5">
              <label className="text-[10px] font-mono uppercase tracking-wider text-neutral-500">
                Target Days/Wk
              </label>
              <select
                value={targetDays}
                onChange={(e) => setTargetDays(Number(e.target.value))}
                className="w-full h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-mono"
              >
                <option value={7}>7 days (Everyday)</option>
                <option value={6}>6 days / week</option>
                <option value={5}>5 days (Weekdays)</option>
                <option value={4}>4 days / week</option>
                <option value={3}>3 days / week</option>
                <option value={2}>2 days / week</option>
                <option value={1}>1 day / week</option>
              </select>
            </div>
          </div>

          {/* Description */}
          <div className="space-y-1.5">
            <label className="text-[10px] font-mono uppercase tracking-wider text-neutral-500">
              Description / Goal (Optional)
            </label>
            <input
              type="text"
              maxLength={200}
              placeholder="e.g., Minimum 3 miles before 8:00 AM"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              className="w-full h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 focus:outline-none focus:border-neutral-500 font-sans"
            />
          </div>

          {/* Actions */}
          <div className="flex items-center justify-end gap-2 pt-3 border-t border-neutral-200 dark:border-neutral-800">
            <button
              type="button"
              onClick={onClose}
              className="h-9 px-3.5 rounded-md text-xs font-medium text-neutral-600 dark:text-neutral-400 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition cursor-pointer"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={isSubmitting}
              className="h-9 px-4 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50 cursor-pointer"
            >
              {isSubmitting ? 'Saving...' : initialData ? 'Save Changes' : 'Create Habit'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
