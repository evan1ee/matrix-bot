import React, { useState, useEffect } from 'react';
import { X, Trash2 } from 'lucide-react';
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
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    if (day) {
      const initialMood = (day.mood?.toLowerCase() as MoodType) || 'good';
      setSelectedMood(initialMood in MOOD_CONFIG ? initialMood : 'good');

      const initialWeather = (day.weather?.toLowerCase() as WeatherType) || null;
      setSelectedWeather(initialWeather && initialWeather in WEATHER_CONFIG ? initialWeather : null);

      setSelectedActivities(day.activities || []);
      setNote(day.note || '');
      setSaveError(null);
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
    setSaveError(null);
    try {
      await onSave({
        date: day.date,
        mood: selectedMood,
        weather: selectedWeather,
        activities: selectedActivities,
        note: note.trim() || null,
      });
      onClose();
    } catch (err: any) {
      console.error('Failed to save day log:', err);
      setSaveError(err?.message || 'Failed to save day record');
    } finally {
      setIsSaving(false);
    }
  };

  const handleDelete = async () => {
    if (!day || !day.mood) return;
    if (window.confirm(`Clear record for ${day.date}?`)) {
      setIsDeleting(true);
      setSaveError(null);
      try {
        await onDelete(day.date);
        onClose();
      } catch (err: any) {
        console.error('Failed to clear day log:', err);
        setSaveError(err?.message || 'Failed to clear day record');
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
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/50 animate-in fade-in">
      <div
        className="bg-white dark:bg-zinc-900 border border-zinc-200 dark:border-zinc-800 rounded-lg w-full max-w-lg overflow-hidden shadow-lg transition-all"
        role="dialog"
        aria-modal="true"
      >
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-zinc-100 dark:border-zinc-800">
          <div>
            <div className="flex items-center gap-2">
              <h3 className="text-base font-semibold text-zinc-900 dark:text-zinc-100">
                {formattedDate}
              </h3>
              {day.day_type === 'present' && (
                <span className="text-[11px] font-medium text-emerald-600 dark:text-emerald-400">
                  (Today)
                </span>
              )}
            </div>
            <p className="text-xs text-zinc-500 dark:text-zinc-400 mt-0.5">
              Record your mood, weather, activities, and notes.
            </p>
          </div>
          <button
            onClick={onClose}
            className="p-1 rounded text-zinc-400 hover:text-zinc-600 dark:hover:text-zinc-200 hover:bg-zinc-100 dark:hover:bg-zinc-800 transition-colors"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Form Body */}
        <form onSubmit={handleSave} className="p-5 space-y-4">
          {/* 1. Primary Mood */}
          <div>
            <label className="block text-xs font-medium text-zinc-600 dark:text-zinc-400 mb-2">
              Mood
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
                    className={`flex flex-col items-center justify-center p-2 rounded-md border text-xs transition-all ${
                      isSelected
                        ? `${config.bgClass} ${config.textClass} font-semibold ring-1 ring-zinc-900 dark:ring-zinc-100 shadow-xs`
                        : 'border-zinc-200 dark:border-zinc-800 bg-zinc-50 dark:bg-zinc-800/50 text-zinc-700 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-800'
                    }`}
                  >
                    <span className="text-xl mb-1">{config.emoji}</span>
                    <span>{config.label}</span>
                  </button>
                );
              })}
            </div>
          </div>

          {/* 2. Weather */}
          <div>
            <label className="block text-xs font-medium text-zinc-600 dark:text-zinc-400 mb-1.5">
              Weather (optional)
            </label>
            <div className="flex flex-wrap gap-1.5">
              {(Object.keys(WEATHER_CONFIG) as WeatherType[]).map((wKey) => {
                const w = WEATHER_CONFIG[wKey];
                const isSelected = selectedWeather === wKey;
                return (
                  <button
                    key={wKey}
                    type="button"
                    onClick={() => setSelectedWeather(isSelected ? null : wKey)}
                    className={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded text-xs font-medium border transition-colors ${
                      isSelected
                        ? 'bg-zinc-900 text-white dark:bg-zinc-100 dark:text-zinc-900 border-zinc-900 dark:border-zinc-100'
                        : 'border-zinc-200 dark:border-zinc-800 bg-zinc-50 dark:bg-zinc-800/40 text-zinc-600 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-800'
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
            <label className="block text-xs font-medium text-zinc-600 dark:text-zinc-400 mb-1.5">
              Activities & milestones
            </label>
            <div className="grid grid-cols-2 sm:grid-cols-3 gap-1.5">
              {ACTIVITY_OPTIONS.map((act) => {
                const isChecked = selectedActivities.includes(act.id);
                return (
                  <button
                    key={act.id}
                    type="button"
                    onClick={() => handleToggleActivity(act.id)}
                    className={`flex items-center gap-1.5 px-2.5 py-1.5 rounded text-xs font-medium border transition-colors text-left ${
                      isChecked
                        ? 'bg-zinc-900 text-white dark:bg-zinc-100 dark:text-zinc-900 border-zinc-900 dark:border-zinc-100'
                        : 'border-zinc-200 dark:border-zinc-800 bg-zinc-50 dark:bg-zinc-800/40 text-zinc-600 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-800'
                    }`}
                  >
                    <span>{act.emoji}</span>
                    <span className="truncate">{act.label}</span>
                  </button>
                );
              })}
            </div>
          </div>

          {/* 4. Reflection Note */}
          <div>
            <label className="block text-xs font-medium text-zinc-600 dark:text-zinc-400 mb-1">
              Notes (optional)
            </label>
            <textarea
              value={note}
              onChange={(e) => setNote(e.target.value)}
              placeholder="What happened today? Thoughts, memory, or highlights..."
              maxLength={500}
              rows={3}
              className="w-full px-3 py-2 rounded-md text-xs border border-zinc-200 dark:border-zinc-700 bg-zinc-50/50 dark:bg-zinc-800/50 text-zinc-900 dark:text-zinc-100 placeholder-zinc-400 focus:outline-hidden focus:ring-1 focus:ring-zinc-900 dark:focus:ring-zinc-100 resize-none"
            />
            <div className="text-right text-[11px] text-zinc-400 mt-0.5 tabular-nums">
              {note.length}/500
            </div>
          </div>

          {/* Error Banner */}
          {saveError && (
            <div className="p-2.5 rounded-md bg-rose-50 dark:bg-rose-950/40 border border-rose-200 dark:border-rose-900 text-xs text-rose-700 dark:text-rose-300 flex items-start gap-2">
              <span className="shrink-0 font-bold">⚠️</span>
              <span className="flex-1">{saveError}</span>
            </div>
          )}

          {/* Footer Actions */}
          <div className="flex items-center justify-between pt-2 border-t border-zinc-100 dark:border-zinc-800">
            {day.mood ? (
              <button
                type="button"
                onClick={handleDelete}
                disabled={isDeleting || isSaving}
                className="inline-flex items-center gap-1.5 text-xs text-rose-600 dark:text-rose-400 hover:underline transition-colors disabled:opacity-50"
              >
                <Trash2 className="w-3.5 h-3.5" />
                <span>{isDeleting ? 'Clearing...' : 'Clear record'}</span>
              </button>
            ) : (
              <div />
            )}

            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={onClose}
                disabled={isSaving || isDeleting}
                className="h-9 px-3.5 text-xs font-medium text-zinc-600 dark:text-zinc-400 hover:text-zinc-900 dark:hover:text-zinc-100 rounded-md transition-colors disabled:opacity-50 cursor-pointer"
              >
                Cancel
              </button>
              <button
                type="submit"
                disabled={isSaving || isDeleting}
                className="h-9 inline-flex items-center gap-1.5 px-4 text-xs font-medium text-white bg-zinc-900 hover:bg-zinc-800 dark:bg-zinc-100 dark:text-zinc-900 dark:hover:bg-white rounded-md shadow-xs transition-colors disabled:opacity-50 cursor-pointer"
              >
                {isSaving && (
                  <svg className="animate-spin -ml-0.5 mr-1 h-3 w-3 text-white dark:text-zinc-900" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24">
                    <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4"></circle>
                    <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
                  </svg>
                )}
                <span>{isSaving ? 'Saving...' : 'Save record'}</span>
              </button>
            </div>
          </div>
        </form>
      </div>
    </div>
  );
};
