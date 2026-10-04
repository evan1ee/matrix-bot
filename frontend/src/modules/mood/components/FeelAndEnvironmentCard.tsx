import React from 'react';
import type { LifeDayItem, MoodType, WeatherType } from '../../life/types';
import { MOOD_CONFIG, WEATHER_CONFIG, ACTIVITY_OPTIONS } from '../../life/types';

export interface FeelAndEnvironmentCardProps {
  isFormActive: boolean;
  dayLog?: LifeDayItem | null;
  selectedMood: MoodType;
  onSelectMood: (mood: MoodType) => void;
  selectedWeather: WeatherType | null;
  onSelectWeather: (weather: WeatherType | null) => void;
  selectedActivities: string[];
  onToggleActivity: (id: string) => void;
}

export const FeelAndEnvironmentCard: React.FC<FeelAndEnvironmentCardProps> = ({
  isFormActive,
  dayLog,
  selectedMood,
  onSelectMood,
  selectedWeather,
  onSelectWeather,
  selectedActivities,
  onToggleActivity,
}) => {
  if (isFormActive) {
    return (
      <>
        {/* Feel / Primary Mood & Weather Card */}
        <div className="border border-neutral-200 dark:border-neutral-800 rounded-lg p-5 bg-white dark:bg-neutral-900 space-y-5">
          <div className="flex items-center justify-between">
            <span className="text-xs font-semibold text-neutral-900 dark:text-neutral-100 font-sans">
              Feel & environment
            </span>
            <span className="text-[10px] font-mono text-neutral-400">
              Record
            </span>
          </div>

          {/* Mood Selector */}
          <div>
            <label className="block text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans mb-2">
              Feel / primary mood
            </label>
            <div className="grid grid-cols-5 gap-2">
              {(Object.keys(MOOD_CONFIG) as MoodType[]).map((moodKey) => {
                const config = MOOD_CONFIG[moodKey];
                const isSelected = selectedMood === moodKey;
                return (
                  <button
                    key={moodKey}
                    type="button"
                    onClick={() => onSelectMood(moodKey)}
                    className={`flex flex-col items-center justify-center p-2.5 rounded-lg border text-xs transition-colors duration-150 cursor-pointer ${
                      isSelected
                        ? `${config.bgClass} ${config.textClass} font-semibold ring-1 ring-neutral-900 dark:ring-neutral-100 shadow-xs`
                        : 'border-neutral-200 dark:border-neutral-800 bg-neutral-50 dark:bg-neutral-800/50 text-neutral-700 dark:text-neutral-300 hover:bg-neutral-100 dark:hover:bg-neutral-800 hover:border-neutral-300 dark:hover:border-neutral-700'
                    }`}
                  >
                    <span className="text-2xl mb-1">{config.emoji}</span>
                    <span className="text-[11px] font-sans">{config.label}</span>
                  </button>
                );
              })}
            </div>
          </div>

          {/* Weather */}
          <div className="pt-4 border-t border-neutral-100 dark:border-neutral-800">
            <label className="block text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans mb-2">
              Weather (optional)
            </label>
            <div className="flex flex-wrap gap-2">
              {(Object.keys(WEATHER_CONFIG) as WeatherType[]).map((wKey) => {
                const w = WEATHER_CONFIG[wKey];
                const isSelected = selectedWeather === wKey;
                return (
                  <button
                    key={wKey}
                    type="button"
                    onClick={() => onSelectWeather(isSelected ? null : wKey)}
                    className={`inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium border transition-colors duration-150 cursor-pointer ${
                      isSelected
                        ? 'bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900 border-neutral-900 dark:border-neutral-100 shadow-2xs'
                        : 'border-neutral-200 dark:border-neutral-800 bg-neutral-50 dark:bg-neutral-800/40 text-neutral-600 dark:text-neutral-400 hover:bg-neutral-100 dark:hover:bg-neutral-800 hover:border-neutral-300 dark:hover:border-neutral-700'
                    }`}
                  >
                    <span>{w.emoji}</span>
                    <span className="font-sans">{w.label}</span>
                  </button>
                );
              })}
            </div>
          </div>
        </div>

        {/* Activities & Milestones Card */}
        <div className="border border-neutral-200 dark:border-neutral-800 rounded-lg p-5 bg-white dark:bg-neutral-900 space-y-3">
          <div className="flex items-center justify-between">
            <span className="text-xs font-semibold text-neutral-900 dark:text-neutral-100 font-sans">
              Activities & milestones
            </span>
            <span className="text-[10px] font-mono text-neutral-400">
              {selectedActivities.length > 0
                ? `${selectedActivities.length} selected`
                : 'Optional'}
            </span>
          </div>
          <div className="grid grid-cols-2 gap-2">
            {ACTIVITY_OPTIONS.map((act) => {
              const isChecked = selectedActivities.includes(act.id);
              return (
                <button
                  key={act.id}
                  type="button"
                  onClick={() => onToggleActivity(act.id)}
                  className={`flex items-center gap-2 px-3 py-2 rounded-md text-xs font-medium border transition-colors duration-150 text-left cursor-pointer ${
                    isChecked
                      ? 'bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900 border-neutral-900 dark:border-neutral-100 shadow-2xs'
                      : 'border-neutral-200 dark:border-neutral-800 bg-neutral-50 dark:bg-neutral-800/40 text-neutral-600 dark:text-neutral-400 hover:bg-neutral-100 dark:hover:bg-neutral-800 hover:border-neutral-300 dark:hover:border-neutral-700'
                  }`}
                >
                  <span>{act.emoji}</span>
                  <span className="truncate font-sans">{act.label}</span>
                </button>
              );
            })}
          </div>
        </div>
      </>
    );
  }

  // Sealed View
  const loggedMoodCfg = dayLog?.mood
    ? MOOD_CONFIG[dayLog.mood.toLowerCase() as MoodType]
    : undefined;
  const loggedWeatherCfg = dayLog?.weather
    ? WEATHER_CONFIG[dayLog.weather.toLowerCase() as WeatherType]
    : undefined;

  return (
    <div className="border border-neutral-200 dark:border-neutral-800 rounded-lg p-5 bg-white dark:bg-neutral-900 space-y-5">
      <div className="flex items-center justify-between">
        <span className="text-xs font-semibold text-neutral-900 dark:text-neutral-100 font-sans">
          Day summary
        </span>
        <span className="text-[10px] font-mono text-emerald-600 dark:text-emerald-400">
          Recorded
        </span>
      </div>

      {/* Mood Banner */}
      <div
        className={`flex items-center gap-3 p-4 rounded-lg border ${
          loggedMoodCfg
            ? `${loggedMoodCfg.bgClass} ${loggedMoodCfg.textClass} font-semibold shadow-xs`
            : 'bg-neutral-50 dark:bg-neutral-800/50 border-neutral-200 dark:border-neutral-800 text-neutral-800 dark:text-neutral-200'
        }`}
      >
        <span className="text-3xl">{loggedMoodCfg?.emoji || '✨'}</span>
        <div>
          <div className="text-base font-bold capitalize font-sans">{loggedMoodCfg?.label || dayLog?.mood}</div>
          <div className="text-xs opacity-80 font-sans">Primary recorded mood</div>
        </div>
      </div>

      {/* Weather Banner */}
      {dayLog?.weather && (
        <div>
          <div className="text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans mb-1.5">
            Weather
          </div>
          <div className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-neutral-50 dark:bg-neutral-800/60 border border-neutral-200 dark:border-neutral-700 text-xs font-medium text-neutral-800 dark:text-neutral-200">
            <span className="text-sm">{loggedWeatherCfg?.emoji || '🌤️'}</span>
            <span className="font-sans">{loggedWeatherCfg?.label || dayLog.weather}</span>
          </div>
        </div>
      )}

      {/* Activities & Milestones Tags */}
      <div>
        <div className="text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans mb-1.5">
          Activities & milestones
        </div>
        {dayLog?.activities && dayLog.activities.length > 0 ? (
          <div className="flex flex-wrap gap-1.5">
            {dayLog.activities.map((actId) => {
              const act = ACTIVITY_OPTIONS.find((a) => a.id === actId);
              return (
                <span
                  key={actId}
                  className="inline-flex items-center gap-1 px-2.5 py-1 rounded-md text-xs font-medium bg-neutral-50 dark:bg-neutral-800/60 border border-neutral-200 dark:border-neutral-700 text-neutral-800 dark:text-neutral-200"
                >
                  <span>{act?.emoji || '📌'}</span>
                  <span className="font-sans">{act?.label || actId}</span>
                </span>
              );
            })}
          </div>
        ) : (
          <p className="text-[11px] text-neutral-400 dark:text-neutral-500 italic font-sans">
            No activities or milestones recorded for this day.
          </p>
        )}
      </div>
    </div>
  );
};
