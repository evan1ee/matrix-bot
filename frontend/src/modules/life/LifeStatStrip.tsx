import React from 'react';
import { Calendar, Heart, Clock, Smile, Settings } from 'lucide-react';
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

  return (
    <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
      {/* Age in Days */}
      <div className="bg-white dark:bg-zinc-900 border border-zinc-200 dark:border-zinc-800/80 rounded-xl p-4 shadow-sm relative group">
        <div className="flex items-center justify-between text-zinc-500 dark:text-zinc-400 mb-1">
          <span className="text-xs font-medium uppercase tracking-wider">Age in Days</span>
          <Clock className="w-4 h-4 text-zinc-400" />
        </div>
        <div className="text-2xl font-bold font-mono tabular-nums text-zinc-900 dark:text-zinc-100">
          {stats.age_in_days.toLocaleString()}
          <span className="text-xs font-normal text-zinc-500 ml-1.5">days</span>
        </div>
        <p className="text-[11px] text-zinc-400 mt-1">Born {stats.birth_date}</p>
      </div>

      {/* % Life Lived */}
      <div className="bg-white dark:bg-zinc-900 border border-zinc-200 dark:border-zinc-800/80 rounded-xl p-4 shadow-sm">
        <div className="flex items-center justify-between text-zinc-500 dark:text-zinc-400 mb-1">
          <span className="text-xs font-medium uppercase tracking-wider">Life Lived</span>
          <Heart className="w-4 h-4 text-zinc-400" />
        </div>
        <div className="text-2xl font-bold font-mono tabular-nums text-zinc-900 dark:text-zinc-100">
          {stats.life_lived_percentage.toFixed(1)}%
        </div>
        <div className="w-full bg-zinc-100 dark:bg-zinc-800 h-1.5 rounded-full overflow-hidden mt-2">
          <div
            className="bg-emerald-500 h-full rounded-full transition-all duration-500"
            style={{ width: `${Math.min(100, Math.max(0, stats.life_lived_percentage))}%` }}
          />
        </div>
      </div>

      {/* Days Logged */}
      <div className="bg-white dark:bg-zinc-900 border border-zinc-200 dark:border-zinc-800/80 rounded-xl p-4 shadow-sm">
        <div className="flex items-center justify-between text-zinc-500 dark:text-zinc-400 mb-1">
          <span className="text-xs font-medium uppercase tracking-wider">Days Logged</span>
          <Calendar className="w-4 h-4 text-zinc-400" />
        </div>
        <div className="text-2xl font-bold font-mono tabular-nums text-zinc-900 dark:text-zinc-100">
          {stats.days_logged.toLocaleString()}
          <span className="text-xs font-normal text-zinc-500 ml-1.5">recorded</span>
        </div>
        <p className="text-[11px] text-zinc-400 mt-1">
          Goal: {stats.expected_lifespan_years} yrs (~{stats.total_life_days.toLocaleString()} days)
        </p>
      </div>

      {/* Dominant Mood & Settings */}
      <div className="bg-white dark:bg-zinc-900 border border-zinc-200 dark:border-zinc-800/80 rounded-xl p-4 shadow-sm flex flex-col justify-between">
        <div className="flex items-center justify-between text-zinc-500 dark:text-zinc-400 mb-1">
          <span className="text-xs font-medium uppercase tracking-wider">Dominant Mood</span>
          <button
            onClick={onOpenSettings}
            title="Edit Life Profile"
            className="text-zinc-400 hover:text-zinc-600 dark:hover:text-zinc-200 p-1 -m-1 rounded-md transition-colors"
          >
            <Settings className="w-4 h-4" />
          </button>
        </div>
        <div className="flex items-center gap-2 mt-1">
          {moodCfg ? (
            <span
              className={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-semibold ${moodCfg.bgClass} ${moodCfg.textClass}`}
            >
              <span>{moodCfg.emoji}</span>
              <span>{moodCfg.label}</span>
            </span>
          ) : (
            <span className="text-sm font-medium text-zinc-400 flex items-center gap-1">
              <Smile className="w-4 h-4" />
              <span>No logs yet</span>
            </span>
          )}
        </div>
        <button
          onClick={onOpenSettings}
          className="text-[11px] text-zinc-400 hover:text-zinc-600 dark:hover:text-zinc-300 transition-colors text-left mt-2"
        >
          Lifespan: {stats.expected_lifespan_years} yrs (click to edit)
        </button>
      </div>
    </div>
  );
};
