import React, { useState, useEffect } from 'react';
import { X, Trash2, Check, Sparkles } from 'lucide-react';
import type { LifeDayItem, MoodType, WeatherType, UpsertDayPayload } from './types';
import { MOOD_CONFIG, WEATHER_CONFIG, ACTIVITY_OPTIONS } from './types';

interface LifeDayModalProps {
  day: LifeDayItem | null;
  isOpen: boolean;
  onClose: () => void;
  onSave: (payload: UpsertDayPayload) => Promise<void>;
  onDelete: (date: string) => Promise<void>;
}

export const LifeDayModal: React.FC<LifeDayModalProps> = ({
  day,
  isOpen,
  onClose,
  onSave,
  onDelete,
}) => {
  const [selectedMood, setSelectedMood] = useState<MoodType>('good');
  const [selectedWeather, setSelectedWeather] = useState<WeatherType | null>(null);
  const [selectedActivities, setSelectedActivities] = useState<string[]>([]);
  const [note, setNote] = useState('');
  const [isSaving, setIsSaving] = useState(false);
  const [isDeleting, setIsDeleting] = useState(false);

  useEffect(() => {
    if (day) {
      const initialMood = (day.mood?.toLowerCase() as MoodType) || 'good';
      setSelectedMood(initialMood in MOOD_CONFIG ? initialMood : 'good');

      const initialWeather = (day.weather?.toLowerCase() as WeatherType) || null;
      setSelectedWeather(initialWeather && initialWeather in WEATHER_CONFIG ? initialWeather : null);

      setSelectedActivities(day.activities || []);
      setNote(day.note || '');
    }
  }, [day]);

  if (!isOpen || !day) return null;

  const handleToggleActivity = (actId: string) => {
    setSelectedActivities((prev) =>
      prev.includes(actId) ? prev.filter((a) => a !== actId) : [...prev, actId]
    );
  };

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!day) return;
    setIsSaving(true);
    try {
      await onSave({
        date: day.date,
        mood: selectedMood,
        weather: selectedWeather,
        activities: selectedActivities,
        note: note.trim() || null,
      });
      onClose();
    } finally {
      setIsSaving(false);
    }
  };

  const handleDelete = async () => {
    if (!day || !day.mood) return;
    if (window.confirm(`Clear record for ${day.date}?`)) {
      setIsDeleting(true);
      try {
        await onDelete(day.date);
        onClose();
      } finally {
        setIsDeleting(false);
      }
    }
  };

  // Format date nicely: e.g. "Friday, Sep 4, 2026"
  const formattedDate = new Date(`${day.date}T12:00:00Z`).toLocaleDateString('en-US', {
    weekday: 'long',
    month: 'short',
    day: 'numeric',
    year: 'numeric',
  });

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs animate-in fade-in">
      <div
        className="bg-white dark:bg-zinc-900 border border-zinc-200 dark:border-zinc-800 rounded-2xl w-full max-w-lg overflow-hidden shadow-2xl transition-all"
        role="dialog"
        aria-modal="true"
      >
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-zinc-100 dark:border-zinc-800">
          <div>
            <div className="flex items-center gap-2">
              <h3 className="text-base font-bold text-zinc-900 dark:text-zinc-100">
                {formattedDate}
              </h3>
              {day.day_type === 'present' && (
                <span className="px-2 py-0.5 rounded-full text-[10px] font-bold bg-emerald-50 text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300 border border-emerald-200 dark:border-emerald-800">
                  TODAY
                </span>
              )}
            </div>
            <p className="text-xs text-zinc-400 mt-0.5">
              Record how this day felt and what made it memorable.
            </p>
          </div>
          <button
            onClick={onClose}
            className="p-1.5 rounded-lg text-zinc-400 hover:text-zinc-600 dark:hover:text-zinc-200 hover:bg-zinc-100 dark:hover:bg-zinc-800 transition-colors"
          >
            <X className="w-5 h-5" />
          </button>
        </div>

        {/* Form Body */}
        <form onSubmit={handleSave} className="p-6 space-y-5">
          {/* 1. Primary Mood */}
          <div>
            <label className="block text-xs font-semibold uppercase tracking-wider text-zinc-500 dark:text-zinc-400 mb-2">
              How was your day? (Mood)
            </label>
            <div className="grid grid-cols-5 gap-2">
              {(Object.keys(MOOD_CONFIG) as MoodType[]).map((moodKey) => {
                const config = MOOD_CONFIG[moodKey];
                const isSelected = selectedMood === moodKey;
                return (
                  <button
                    key={moodKey}
                    type="button"
                    onClick={() => setSelectedMood(moodKey)}
                    className={`flex flex-col items-center justify-center p-2.5 rounded-xl border transition-all ${
                      isSelected
                        ? `${config.bgClass} ${config.textClass} font-bold ring-2 ring-offset-2 dark:ring-offset-zinc-900 ${config.borderClass} scale-105 shadow-sm`
                        : 'border-zinc-200 dark:border-zinc-800 bg-zinc-50/50 dark:bg-zinc-800/40 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-800'
                    }`}
                  >
                    <span className="text-2xl mb-1">{config.emoji}</span>
                    <span className="text-xs">{config.label}</span>
                  </button>
                );
              })}
            </div>
          </div>

          {/* 2. Weather */}
          <div>
            <label className="block text-xs font-semibold uppercase tracking-wider text-zinc-500 dark:text-zinc-400 mb-2">
              Weather (Optional)
            </label>
            <div className="flex flex-wrap gap-2">
              {(Object.keys(WEATHER_CONFIG) as WeatherType[]).map((wKey) => {
                const w = WEATHER_CONFIG[wKey];
                const isSelected = selectedWeather === wKey;
                return (
                  <button
                    key={wKey}
                    type="button"
                    onClick={() => setSelectedWeather(isSelected ? null : wKey)}
                    className={`inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium border transition-all ${
                      isSelected
                        ? 'bg-zinc-900 text-white dark:bg-white dark:text-zinc-900 border-zinc-900 dark:border-white shadow-xs'
                        : 'border-zinc-200 dark:border-zinc-800 bg-zinc-50/50 dark:bg-zinc-800/40 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-800'
                    }`}
                  >
                    <span>{w.emoji}</span>
                    <span>{w.label}</span>
                  </button>
                );
              })}
            </div>
          </div>

          {/* 3. Activities / Checkboxes */}
          <div>
            <label className="block text-xs font-semibold uppercase tracking-wider text-zinc-500 dark:text-zinc-400 mb-2">
              Activities & Milestones
            </label>
            <div className="grid grid-cols-2 sm:grid-cols-3 gap-2">
              {ACTIVITY_OPTIONS.map((act) => {
                const isChecked = selectedActivities.includes(act.id);
                return (
                  <button
                    key={act.id}
                    type="button"
                    onClick={() => handleToggleActivity(act.id)}
                    className={`flex items-center gap-1.5 px-2.5 py-2 rounded-lg text-xs font-medium border transition-all text-left ${
                      isChecked
                        ? 'bg-sky-50 dark:bg-sky-950/60 border-sky-300 dark:border-sky-800 text-sky-900 dark:text-sky-200'
                        : 'border-zinc-200 dark:border-zinc-800 bg-zinc-50/50 dark:bg-zinc-800/40 text-zinc-600 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-800'
                    }`}
                  >
                    <span>{act.emoji}</span>
                    <span className="truncate">{act.label}</span>
                    {isChecked && <Check className="w-3.5 h-3.5 ml-auto text-sky-500" />}
                  </button>
                );
              })}
            </div>
          </div>

          {/* 4. Reflection Note */}
          <div>
            <label className="block text-xs font-semibold uppercase tracking-wider text-zinc-500 dark:text-zinc-400 mb-1.5">
              Reflection / Notes (Optional)
            </label>
            <textarea
              value={note}
              onChange={(e) => setNote(e.target.value)}
              placeholder="What happened today? Thoughts, memory, or highlights..."
              maxLength={500}
              rows={3}
              className="w-full px-3 py-2 rounded-xl text-sm border border-zinc-200 dark:border-zinc-700 bg-zinc-50/50 dark:bg-zinc-800/60 text-zinc-900 dark:text-zinc-100 placeholder-zinc-400 focus:outline-hidden focus:ring-2 focus:ring-zinc-900 dark:focus:ring-zinc-100 resize-none"
            />
            <div className="text-right text-[11px] text-zinc-400 mt-0.5">
              {note.length}/500
            </div>
          </div>

          {/* Footer Actions */}
          <div className="flex items-center justify-between pt-2 border-t border-zinc-100 dark:border-zinc-800">
            {day.mood ? (
              <button
                type="button"
                onClick={handleDelete}
                disabled={isDeleting}
                className="inline-flex items-center gap-1.5 px-3 py-2 text-xs font-medium text-rose-600 dark:text-rose-400 hover:bg-rose-50 dark:hover:bg-rose-950/40 rounded-lg transition-colors"
              >
                <Trash2 className="w-4 h-4" />
                <span>{isDeleting ? 'Clearing...' : 'Clear Entry'}</span>
              </button>
            ) : (
              <div />
            )}

            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={onClose}
                className="px-4 py-2 text-xs font-medium text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-800 rounded-lg transition-colors"
              >
                Cancel
              </button>
              <button
                type="submit"
                disabled={isSaving}
                className="inline-flex items-center gap-1.5 px-4 py-2 text-xs font-semibold text-white bg-zinc-900 hover:bg-zinc-800 dark:bg-zinc-100 dark:text-zinc-900 dark:hover:bg-white rounded-lg shadow-xs transition-colors"
              >
                <Sparkles className="w-3.5 h-3.5" />
                <span>{isSaving ? 'Saving...' : 'Save Record'}</span>
              </button>
            </div>
          </div>
        </form>
      </div>
    </div>
  );
};
