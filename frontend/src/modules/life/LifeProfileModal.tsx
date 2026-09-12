import React, { useState, useEffect } from 'react';
import { X } from 'lucide-react';
import type { LifeProfile, UpsertProfilePayload } from './types';
import { DatePicker } from '../../components/DatePicker';

interface LifeProfileModalProps {
  profile: LifeProfile | null;
  isOpen: boolean;
  onClose: () => void;
  onSave: (payload: UpsertProfilePayload) => Promise<void>;
}

export const LifeProfileModal: React.FC<LifeProfileModalProps> = ({
  profile,
  isOpen,
  onClose,
  onSave,
}) => {
  const [birthDate, setBirthDate] = useState('1995-01-01');
  const [lifespan, setLifespan] = useState(80);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (profile) {
      setBirthDate(profile.birth_date);
      setLifespan(profile.expected_lifespan_years || 80);
    }
  }, [profile]);

  if (!isOpen) return null;

  const totalDays = Math.floor(lifespan * 365.25);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    if (!birthDate) {
      setError('Please select your birth date');
      return;
    }

    setIsSaving(true);
    try {
      await onSave({
        birth_date: birthDate,
        expected_lifespan_years: lifespan,
      });
      onClose();
    } catch (err: any) {
      setError(err?.message || 'Failed to save life profile');
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/50 animate-in fade-in">
      <div
        className="bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded-lg w-full max-w-md overflow-hidden shadow-lg transition-all"
        role="dialog"
        aria-modal="true"
      >
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-neutral-100 dark:border-neutral-800">
          <div>
            <h3 className="text-base font-semibold text-neutral-900 dark:text-neutral-100">
              {profile ? 'Edit life profile' : 'Set up life grid'}
            </h3>
            <p className="text-xs text-neutral-500 dark:text-neutral-400 mt-0.5">
              Visualize every single day of your lifespan.
            </p>
          </div>
          {profile && (
            <button
              onClick={onClose}
              className="p-1 rounded text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-200 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition-colors"
            >
              <X className="w-4 h-4" />
            </button>
          )}
        </div>

        {/* Form Body */}
        <form onSubmit={handleSubmit} className="p-5 space-y-4">
          {error && (
            <div className="p-2.5 rounded text-xs bg-rose-50 text-rose-700 dark:bg-rose-950/40 dark:text-rose-300 border border-rose-200 dark:border-rose-900">
              {error}
            </div>
          )}

          {/* Birth Date */}
          <div>
            <label className="block text-xs font-medium text-neutral-600 dark:text-neutral-400 mb-1">
              Date of birth
            </label>
            <DatePicker
              value={birthDate}
              onChange={setBirthDate}
              required
              max={new Date().toISOString().split('T')[0]}
              placeholder="Select date of birth"
            />
            <p className="text-xs text-neutral-400 dark:text-neutral-500 mt-1">
              Anchors your timeline from Day 1.
            </p>
          </div>

          {/* Expected Lifespan */}
          <div>
            <div className="flex items-center justify-between mb-1">
              <label className="text-xs font-medium text-neutral-600 dark:text-neutral-400">
                Assumed lifespan
              </label>
              <span className="text-xs font-mono font-medium text-neutral-900 dark:text-neutral-100 tabular-nums">
                {lifespan} years
              </span>
            </div>
            <input
              type="range"
              min={40}
              max={120}
              step={1}
              value={lifespan}
              onChange={(e) => setLifespan(Number(e.target.value))}
              className="w-full accent-neutral-900 dark:accent-neutral-100 cursor-pointer"
            />
            <div className="flex justify-between text-xs text-neutral-400 dark:text-neutral-500 mt-1 tabular-nums">
              <span>40 yrs</span>
              <span className="font-mono">
                ~{totalDays.toLocaleString()} days
              </span>
              <span>120 yrs</span>
            </div>
          </div>

          {/* Footer Actions */}
          <div className="flex items-center justify-end gap-2 pt-2 border-t border-neutral-100 dark:border-neutral-800">
            {profile && (
              <button
                type="button"
                onClick={onClose}
                className="h-9 px-3.5 text-xs font-medium text-neutral-600 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 rounded-md transition-colors cursor-pointer"
              >
                Cancel
              </button>
            )}
            <button
              type="submit"
              disabled={isSaving}
              className="h-9 px-4 text-xs font-medium text-white bg-neutral-900 hover:bg-neutral-800 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-white rounded-md shadow-xs transition-colors cursor-pointer"
            >
              {isSaving ? 'Saving...' : 'Save profile'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
