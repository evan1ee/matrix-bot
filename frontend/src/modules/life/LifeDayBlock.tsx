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
    if (moodCfg) {
      stateClasses = `${moodCfg.bgClass} ${moodCfg.textClass} font-semibold ring-2 ring-blue-500 ring-offset-1 dark:ring-offset-neutral-950 shadow-xs`;
    } else {
      stateClasses =
        'bg-blue-600 dark:bg-blue-500 text-white font-bold ring-2 ring-blue-400/50';
    }
  } else if (moodCfg) {
    stateClasses = `${moodCfg.bgClass} ${moodCfg.textClass} font-semibold shadow-xs`;
  } else if (day_type === 'past') {
    stateClasses =
      'bg-neutral-100 dark:bg-neutral-800/70 text-neutral-500 dark:text-neutral-400 hover:bg-neutral-200 dark:hover:bg-neutral-700/80';
  } else {
    // Future: clean white or subtle dark
    stateClasses =
      'bg-white dark:bg-neutral-900/60 text-neutral-400 dark:text-neutral-600 border border-neutral-200 dark:border-neutral-800/80 hover:border-neutral-300 dark:hover:border-neutral-700';
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
        <span className="absolute bottom-0.5 w-1 h-1 rounded-full bg-neutral-400 dark:bg-neutral-500" />
      )}
    </button>
  );
};
