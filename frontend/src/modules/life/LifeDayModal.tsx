import React, { useEffect, useMemo } from 'react';
import { X } from 'lucide-react';
import type { LifeDayItem, MoodType, WeatherType } from './types';
import { MOOD_CONFIG, WEATHER_CONFIG, ACTIVITY_OPTIONS, TIME_CATEGORIES } from './types';
import { LifeActivityFeed } from './LifeActivityFeed';

interface LifeDayModalProps {
  day: LifeDayItem | null;
  isOpen: boolean;
  onClose: () => void;
}

export const LifeDayModal: React.FC<LifeDayModalProps> = ({
  day,
  isOpen,
  onClose,
}) => {
  const isLogged = Boolean(day?.mood);
  const loggedMoodCfg = day?.mood ? MOOD_CONFIG[day.mood.toLowerCase() as MoodType] : undefined;
  const loggedWeatherCfg = day?.weather ? WEATHER_CONFIG[day.weather.toLowerCase() as WeatherType] : undefined;

  // Compute category hour totals from time_allocation if present
  const timeSummary = useMemo(() => {
    if (!day?.time_allocation || !Array.isArray(day.time_allocation)) return null;
    const counts: Record<string, number> = {};
    let totalBlocks = 0;
    for (const catId of day.time_allocation) {
      if (catId) {
        counts[catId] = (counts[catId] || 0) + 1;
        totalBlocks++;
      }
    }
    if (totalBlocks === 0) return null;

    const blockHours = day.time_allocation.length <= 96 ? 0.25 : 5 / 60;
    const breakdown = Object.entries(counts).map(([catId, blocks]) => {
      const cfg = TIME_CATEGORIES[catId as keyof typeof TIME_CATEGORIES];
      return {
        id: catId,
        label: cfg?.label || catId,
        emoji: cfg?.emoji || '⏱️',
        hours: (blocks * blockHours).toFixed(1),
      };
    });

    return {
      totalHours: (totalBlocks * blockHours).toFixed(1),
      breakdown,
    };
  }, [day?.time_allocation]);

  // Keyboard accessibility: Escape closes modal
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && isOpen) {
        onClose();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onClose]);

  if (!isOpen || !day) return null;

  const formattedDate = new Date(`${day.date}T12:00:00Z`).toLocaleDateString('en-US', {
    weekday: 'long',
    month: 'short',
    day: 'numeric',
    year: 'numeric',
  });

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/50 animate-in fade-in">
      <div
        className="bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded-lg w-full max-w-4xl max-h-[90vh] overflow-y-auto shadow-lg transition-all"
        role="dialog"
        aria-modal="true"
      >
        {/* Header */}
        <div className="flex items-center justify-between px-5 sm:px-6 py-4 border-b border-neutral-100 dark:border-neutral-800">
          <div>
            <div className="flex items-center gap-2">
              <h3 className="text-base font-semibold text-neutral-900 dark:text-neutral-100">
                {formattedDate}
              </h3>
              {day.day_type === 'present' && (
                <span className="text-[10px] font-mono font-medium px-2 py-0.5 rounded bg-blue-50 dark:bg-blue-950/60 text-blue-700 dark:text-blue-400 border border-blue-200 dark:border-blue-900/60">
                  Today
                </span>
              )}
              {isLogged ? (
                <span className="text-[10px] font-mono font-medium px-2 py-0.5 rounded bg-emerald-50 dark:bg-emerald-950/60 text-emerald-700 dark:text-emerald-400 border border-emerald-200 dark:border-emerald-900/60">
                  Logged
                </span>
              ) : (
                <span className="text-[10px] font-mono font-medium px-2 py-0.5 rounded bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400 border border-neutral-200 dark:border-neutral-700">
                  Auto-Tracked
                </span>
              )}
            </div>
            <p className="text-xs text-neutral-500 dark:text-neutral-400 mt-0.5">
              Life Grid view-only retrospective for this day.
            </p>
          </div>
          <button
            onClick={onClose}
            className="p-1 rounded text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-200 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition-colors cursor-pointer"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Modal Body: Retrospective 2-Column Inspection */}
        <div className="p-5 sm:p-6">
          <div className="grid grid-cols-1 md:grid-cols-12 gap-6 items-start">
            {/* Left Column: Logged Mood, Notes & Time Allocation Summary */}
            <div className="md:col-span-6 space-y-4">
              {/* 1. Mood Card */}
              {isLogged && loggedMoodCfg ? (
                <div>
                  <div className="text-[10px] font-mono font-medium uppercase tracking-wider text-neutral-500 dark:text-neutral-400 mb-1.5">
                    Mood
                  </div>
                  <div
                    className={`flex items-center gap-3 p-3.5 rounded-lg border ${loggedMoodCfg.bgClass} ${loggedMoodCfg.textClass} font-semibold shadow-xs`}
                  >
                    <span className="text-3xl">{loggedMoodCfg.emoji}</span>
                    <div>
                      <div className="text-sm font-bold capitalize">{loggedMoodCfg.label}</div>
                      <div className="text-[11px] opacity-80">Recorded mood for this day</div>
                    </div>
                  </div>
                </div>
              ) : (
                <div className="p-3.5 rounded-lg border border-dashed border-neutral-200 dark:border-neutral-800 text-xs text-neutral-500 flex items-center justify-between">
                  <span>No subjective mood was logged for this day.</span>
                </div>
              )}

              {/* 2. Weather & Milestones */}
              <div className="flex flex-wrap gap-2">
                {day.weather && (
                  <div className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-neutral-50 dark:bg-neutral-800/60 border border-neutral-200 dark:border-neutral-700 text-xs font-medium text-neutral-800 dark:text-neutral-200">
                    <span className="text-sm">{loggedWeatherCfg?.emoji || '🌤️'}</span>
                    <span>{loggedWeatherCfg?.label || day.weather}</span>
                  </div>
                )}
                {day.activities &&
                  day.activities.map((actId) => {
                    const act = ACTIVITY_OPTIONS.find((a) => a.id === actId);
                    return (
                      <span
                        key={actId}
                        className="inline-flex items-center gap-1 px-2.5 py-1 rounded-md text-xs font-medium bg-neutral-50 dark:bg-neutral-800/60 border border-neutral-200 dark:border-neutral-700 text-neutral-800 dark:text-neutral-200"
                      >
                        <span>{act?.emoji || '📌'}</span>
                        <span>{act?.label || actId}</span>
                      </span>
                    );
                  })}
              </div>

              {/* 3. Reflection Note */}
              <div>
                <div className="text-[10px] font-mono font-medium uppercase tracking-wider text-neutral-500 dark:text-neutral-400 mb-1">
                  Notes & Reflection
                </div>
                {day.note ? (
                  <div className="p-3.5 rounded-lg bg-neutral-50 dark:bg-neutral-800/50 border border-neutral-200 dark:border-neutral-800 text-xs text-neutral-800 dark:text-neutral-200 whitespace-pre-wrap leading-relaxed">
                    "{day.note}"
                  </div>
                ) : (
                  <div className="p-3 rounded-md bg-neutral-50/50 dark:bg-neutral-800/30 border border-neutral-200/80 dark:border-neutral-800/80 text-xs text-neutral-400 dark:text-neutral-500 italic">
                    No reflection note recorded.
                  </div>
                )}
              </div>

              {/* 4. Time Allocation Breakdown */}
              {timeSummary && (
                <div>
                  <div className="text-[10px] font-mono font-medium uppercase tracking-wider text-neutral-500 dark:text-neutral-400 mb-1.5 flex items-center justify-between">
                    <span>24h Time Allocation</span>
                    <span className="font-mono text-neutral-700 dark:text-neutral-300 font-semibold">
                      {timeSummary.totalHours}h tracked
                    </span>
                  </div>
                  <div className="p-3 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/40 dark:bg-neutral-800/30 space-y-1.5">
                    <div className="flex flex-wrap gap-2">
                      {timeSummary.breakdown.map((item) => (
                        <div
                          key={item.id}
                          className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded text-xs bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800"
                        >
                          <span>{item.emoji}</span>
                          <span className="text-neutral-700 dark:text-neutral-300">{item.label}</span>
                          <span className="font-mono font-medium text-neutral-900 dark:text-neutral-100">
                            {item.hours}h
                          </span>
                        </div>
                      ))}
                    </div>
                  </div>
                </div>
              )}
            </div>

            {/* Right Column: Sourced Evidence Auto-Logged Activity Feed */}
            <div className="md:col-span-6 space-y-4 md:border-l md:border-neutral-100 md:dark:border-neutral-800 md:pl-6">
              <LifeActivityFeed date={day.date} />
            </div>
          </div>

          {/* Footer Actions */}
          <div className="flex items-center justify-end pt-4 mt-6 border-t border-neutral-100 dark:border-neutral-800">
            <button
              type="button"
              onClick={onClose}
              className="h-8 px-3.5 text-xs font-medium text-white bg-neutral-900 hover:bg-neutral-800 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-white rounded-md shadow-xs transition-colors cursor-pointer"
            >
              Close
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};
