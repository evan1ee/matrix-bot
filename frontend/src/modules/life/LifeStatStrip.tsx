import React from 'react';
import { Settings } from 'lucide-react';
import type { LifeProfileStats, MoodType } from './types';
import { MOOD_CONFIG } from './types';

interface LifeStatStripProps {
  stats: LifeProfileStats | null;
  onOpenSettings: () => void;
}

export const LifeStatStrip: React.FC<LifeStatStripProps> = ({ stats, onOpenSettings }) => {
  if (!stats) return null;

  const dominantMoodKey = (stats.dominant_mood?.toLowerCase() || '') as MoodType;
  const moodCfg = MOOD_CONFIG[dominantMoodKey];
  const daysRemaining = Math.max(0, stats.total_life_days - stats.age_in_days);

  return (
    <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
      {/* 1. Days Elapsed */}
      <div className="bg-white dark:bg-zinc-900 border border-zinc-200 dark:border-zinc-800 rounded-lg p-4 flex flex-col justify-between">
        <div>
          <p className="text-xs font-medium text-zinc-500 dark:text-zinc-400">
            Days Elapsed
          </p>
          <p className="text-2xl font-semibold tracking-tight tabular-nums text-zinc-900 dark:text-zinc-100 mt-1">
            {stats.age_in_days.toLocaleString()}
          </p>
        </div>
        <p className="text-xs text-zinc-400 dark:text-zinc-500 mt-2">
          Started {stats.birth_date}
        </p>
      </div>

      {/* 2. Days Remaining / Ahead */}
      <div className="bg-white dark:bg-zinc-900 border border-zinc-200 dark:border-zinc-800 rounded-lg p-4 flex flex-col justify-between">
        <div>
          <p className="text-xs font-medium text-zinc-500 dark:text-zinc-400">
            Days Ahead
          </p>
          <p className="text-2xl font-semibold tracking-tight tabular-nums text-zinc-900 dark:text-zinc-100 mt-1">
            {daysRemaining.toLocaleString()}
          </p>
        </div>
        <div className="mt-2 space-y-1">
          <div className="w-full bg-zinc-100 dark:bg-zinc-800 h-1.5 rounded-full overflow-hidden">
            <div
              className="bg-zinc-900 dark:bg-zinc-100 h-full rounded-full transition-all duration-300"
              style={{ width: `${Math.min(100, Math.max(0, stats.life_lived_percentage))}%` }}
            />
          </div>
          <p className="text-[11px] text-zinc-400 dark:text-zinc-500 tabular-nums">
            {stats.life_lived_percentage.toFixed(1)}% lived
          </p>
        </div>
      </div>

      {/* 3. Days Logged */}
      <div className="bg-white dark:bg-zinc-900 border border-zinc-200 dark:border-zinc-800 rounded-lg p-4 flex flex-col justify-between">
        <div>
          <p className="text-xs font-medium text-zinc-500 dark:text-zinc-400">
            Days Logged
          </p>
          <p className="text-2xl font-semibold tracking-tight tabular-nums text-zinc-900 dark:text-zinc-100 mt-1">
            {stats.days_logged.toLocaleString()}
          </p>
        </div>
        <div className="mt-2 flex items-center gap-1.5 text-xs text-zinc-400 dark:text-zinc-500">
          <span>Out of ~{stats.total_life_days.toLocaleString()} days</span>
          {(stats.total_deep_work_hours ?? 0) > 0 && (
            <>
              <span>·</span>
              <span className="text-zinc-700 dark:text-zinc-300 font-mono font-medium">
                ⚡ {(stats.total_deep_work_hours ?? 0).toFixed(1)}h
              </span>
            </>
          )}
        </div>
      </div>

      {/* 4. Dominant Mood & Settings */}
      <div className="bg-white dark:bg-zinc-900 border border-zinc-200 dark:border-zinc-800 rounded-lg p-4 flex flex-col justify-between">
        <div className="flex items-center justify-between">
          <p className="text-xs font-medium text-zinc-500 dark:text-zinc-400">
            Dominant Mood
          </p>
          <button
            onClick={onOpenSettings}
            title="Edit profile"
            className="text-zinc-400 hover:text-zinc-600 dark:hover:text-zinc-200 p-0.5 rounded transition-colors"
          >
            <Settings className="w-3.5 h-3.5" />
          </button>
        </div>
        <div className="mt-1">
          {moodCfg ? (
            <div className="flex items-center gap-1.5">
              <span className={`w-2.5 h-2.5 rounded-full ${moodCfg.bgClass}`} />
              <span className="text-base font-semibold text-zinc-900 dark:text-zinc-100">
                {moodCfg.label}
              </span>
              <span className="text-sm">{moodCfg.emoji}</span>
            </div>
          ) : (
            <p className="text-sm text-zinc-400">No records yet</p>
          )}
        </div>
        <div className="flex items-center justify-between text-xs text-zinc-400 dark:text-zinc-500 mt-2">
          <button
            onClick={onOpenSettings}
            className="hover:text-zinc-600 dark:hover:text-zinc-300 text-left transition-colors"
          >
            Lifespan: {stats.expected_lifespan_years} years
          </button>
          {(stats.average_sleep_hours ?? 0) > 0 && (
            <span className="font-mono text-zinc-700 dark:text-zinc-300 font-medium">
              💤 {(stats.average_sleep_hours ?? 0).toFixed(1)}h
            </span>
          )}
        </div>
      </div>
    </div>
  );
};
