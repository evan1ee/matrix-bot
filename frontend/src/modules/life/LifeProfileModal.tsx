import React, { useState, useEffect } from 'react';
import { X, Sparkles, Calendar, Heart } from 'lucide-react';
import type { LifeProfile, UpsertProfilePayload } from './types';

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
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs animate-in fade-in">
      <div
        className="bg-white dark:bg-zinc-900 border border-zinc-200 dark:border-zinc-800 rounded-2xl w-full max-w-md overflow-hidden shadow-2xl transition-all"
        role="dialog"
        aria-modal="true"
      >
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-zinc-100 dark:border-zinc-800">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-lg bg-emerald-50 text-emerald-600 dark:bg-emerald-950/60 dark:text-emerald-400 flex items-center justify-center">
              <Sparkles className="w-4 h-4" />
            </div>
            <div>
              <h3 className="text-base font-bold text-zinc-900 dark:text-zinc-100">
                {profile ? 'Edit Life Profile' : 'Set Up Life Grid'}
              </h3>
              <p className="text-xs text-zinc-400">
                Visualize every single day of your lifespan.
              </p>
            </div>
          </div>
          {profile && (
            <button
              onClick={onClose}
              className="p-1.5 rounded-lg text-zinc-400 hover:text-zinc-600 dark:hover:text-zinc-200 hover:bg-zinc-100 dark:hover:bg-zinc-800 transition-colors"
            >
              <X className="w-5 h-5" />
            </button>
          )}
        </div>

        {/* Form Body */}
        <form onSubmit={handleSubmit} className="p-6 space-y-5">
          {error && (
            <div className="p-3 rounded-lg bg-rose-50 text-rose-700 dark:bg-rose-950/40 dark:text-rose-300 text-xs border border-rose-200 dark:border-rose-900">
              {error}
            </div>
          )}

          {/* Birth Date */}
          <div>
            <label className="block text-xs font-semibold uppercase tracking-wider text-zinc-500 dark:text-zinc-400 mb-1.5 flex items-center gap-1.5">
              <Calendar className="w-3.5 h-3.5" />
              <span>Your Date of Birth</span>
            </label>
            <input
              type="date"
              value={birthDate}
              onChange={(e) => setBirthDate(e.target.value)}
              required
              max={new Date().toISOString().split('T')[0]}
              className="w-full px-3 py-2 rounded-xl text-sm border border-zinc-200 dark:border-zinc-700 bg-zinc-50/50 dark:bg-zinc-800/60 text-zinc-900 dark:text-zinc-100 focus:outline-hidden focus:ring-2 focus:ring-zinc-900 dark:focus:ring-zinc-100"
            />
            <p className="text-[11px] text-zinc-400 mt-1">
              Used to anchor your timeline from Day 1 to the present.
            </p>
          </div>

          {/* Expected Lifespan */}
          <div>
            <div className="flex items-center justify-between mb-1.5">
              <label className="text-xs font-semibold uppercase tracking-wider text-zinc-500 dark:text-zinc-400 flex items-center gap-1.5">
                <Heart className="w-3.5 h-3.5" />
                <span>Assumed Lifespan</span>
              </label>
              <span className="text-sm font-bold font-mono text-zinc-900 dark:text-zinc-100">
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
              className="w-full accent-emerald-500 cursor-pointer"
            />
            <div className="flex justify-between text-[11px] text-zinc-400 mt-1">
              <span>40 yrs</span>
              <span className="font-mono text-zinc-600 dark:text-zinc-300">
                ~{totalDays.toLocaleString()} days
              </span>
              <span>120 yrs</span>
            </div>
          </div>

          {/* Life Horizon Banner preview */}
          <div className="p-3 rounded-xl bg-zinc-50 dark:bg-zinc-800/50 border border-zinc-100 dark:border-zinc-800 text-xs text-zinc-600 dark:text-zinc-400">
            <p className="leading-relaxed">
              Every day will be represented as one block. Future days remain clean white, today is a
              pulsing beacon, and your memories color the past.
            </p>
          </div>

          {/* Footer Actions */}
          <div className="flex items-center justify-end gap-2 pt-2 border-t border-zinc-100 dark:border-zinc-800">
            {profile && (
              <button
                type="button"
                onClick={onClose}
                className="px-4 py-2 text-xs font-medium text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-800 rounded-lg transition-colors"
              >
                Cancel
              </button>
            )}
            <button
              type="submit"
              disabled={isSaving}
              className="inline-flex items-center gap-1.5 px-4 py-2 text-xs font-semibold text-white bg-zinc-900 hover:bg-zinc-800 dark:bg-zinc-100 dark:text-zinc-900 dark:hover:bg-white rounded-lg shadow-xs transition-colors"
            >
              <Sparkles className="w-3.5 h-3.5" />
              <span>{isSaving ? 'Saving...' : 'Start Visualizing Life'}</span>
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
