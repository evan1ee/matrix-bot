import React from 'react';
import type { LifeDayItem, MoodType, WeatherType } from './types';
import { MOOD_CONFIG, WEATHER_CONFIG } from './types';

interface LifeDayBlockProps {
  day: LifeDayItem;
  dayNumber: number;
  isSelected?: boolean;
  onClick: (day: LifeDayItem) => void;
}

export const LifeDayBlock: React.FC<LifeDayBlockProps> = ({
  day,
  dayNumber,
  isSelected,
  onClick,
}) => {
  const { date, day_type, mood, weather, activities, note } = day;
  const moodKey = (mood?.toLowerCase() || '') as MoodType;
  const moodCfg = MOOD_CONFIG[moodKey];
  const weatherKey = (weather?.toLowerCase() || '') as WeatherType;
  const weatherCfg = WEATHER_CONFIG[weatherKey];

  // Base styling for each state
  let stateClasses = '';
  if (day_type === 'present') {
    stateClasses =
      'bg-emerald-600 dark:bg-emerald-500 text-white font-bold ring-2 ring-emerald-500/40';
  } else if (moodCfg) {
    stateClasses = `${moodCfg.bgClass} ${moodCfg.textClass} font-semibold shadow-xs`;
  } else if (day_type === 'past') {
    stateClasses =
      'bg-zinc-100 dark:bg-zinc-800/70 text-zinc-500 dark:text-zinc-400 hover:bg-zinc-200 dark:hover:bg-zinc-700/80';
  } else {
    // Future: clean white or subtle dark
    stateClasses =
      'bg-white dark:bg-zinc-900/60 text-zinc-400 dark:text-zinc-600 border border-zinc-200 dark:border-zinc-800/80 hover:border-zinc-300 dark:hover:border-zinc-700';
  }

  const hasExtra = !!(activities && activities.length > 0) || !!note || !!weather;

  return (
    <button
      type="button"
      onClick={() => onClick(day)}
      title={`${date}${mood ? ` • ${mood}` : ''}${weather ? ` • ${weather}` : ''}${note ? ` • "${note}"` : ''}`}
      className={`relative group aspect-square flex flex-col items-center justify-center rounded-md text-[11px] transition-colors duration-150 select-none ${stateClasses} ${
        isSelected ? 'ring-2 ring-neutral-900 dark:ring-neutral-100 z-10' : ''
      }`}
    >
      <span className="leading-none">{dayNumber}</span>

      {/* Mood Emoji or Weather indicator if logged */}
      {moodCfg && (
        <span className="text-[10px] leading-none mt-0.5 scale-90 sm:scale-100">
          {moodCfg.emoji}
        </span>
      )}

      {/* Mini dot indicator for extra details (note / activity / weather) when no mood or secondary */}
      {!moodCfg && hasExtra && (
        <span className="absolute bottom-0.5 w-1 h-1 rounded-full bg-zinc-400 dark:bg-zinc-500" />
      )}
    </button>
  );
};
