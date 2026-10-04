import React from 'react';
import { Smile, ChevronLeft, ChevronRight, Sparkles } from 'lucide-react';
import { DatePicker } from '../../../components/DatePicker';
import { triggerAiPrompt } from '../../ai';

export interface MoodHeaderProps {
  formattedDate: string;
  selectedDate: string;
  todayStr: string;
  isLogged: boolean;
  onShiftDate: (days: number) => void;
  onSelectDate: (date: string) => void;
  onSelectToday: () => void;
}

export const MoodHeader: React.FC<MoodHeaderProps> = ({
  formattedDate,
  selectedDate,
  todayStr,
  isLogged,
  onShiftDate,
  onSelectDate,
  onSelectToday,
}) => {
  return (
    <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 border-b border-neutral-200 dark:border-neutral-800 pb-4">
      <div className="flex items-center gap-2.5">
        <Smile className="w-4 h-4 text-neutral-600 dark:text-neutral-300" />
        <h2 className="text-base font-semibold text-neutral-900 dark:text-neutral-100 font-sans">
          {formattedDate}
        </h2>
        {selectedDate === todayStr && (
          <span className="text-[10px] font-mono font-medium px-2 py-0.5 rounded bg-blue-50 dark:bg-blue-950/60 text-blue-700 dark:text-blue-400 border border-blue-200 dark:border-blue-900/60">
            Today
          </span>
        )}
        {isLogged && (
          <span className="text-[10px] font-mono font-medium px-2 py-0.5 rounded bg-emerald-50 dark:bg-emerald-950/60 text-emerald-700 dark:text-emerald-400 border border-emerald-200 dark:border-emerald-900/60">
            Logged
          </span>
        )}
      </div>

      {/* Date Navigator Controls & Fast Action Bridge */}
      <div className="flex items-center gap-2">
        <div className="flex items-center gap-1">
          <button
            type="button"
            onClick={() => onShiftDate(-1)}
            className="h-8 w-8 flex items-center justify-center rounded-md border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 hover:bg-neutral-50 dark:hover:bg-neutral-800 hover:border-neutral-300 dark:hover:border-neutral-700 text-neutral-700 dark:text-neutral-300 transition-colors duration-150 cursor-pointer shrink-0 shadow-2xs"
            title="Previous day"
          >
            <ChevronLeft className="w-4 h-4" />
          </button>

          <DatePicker
            value={selectedDate}
            onChange={(val) => val && onSelectDate(val)}
            size="sm"
            align="right"
            required
            className="w-34 sm:w-36 shrink-0"
          />

          <button
            type="button"
            onClick={() => onShiftDate(1)}
            className="h-8 w-8 flex items-center justify-center rounded-md border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 hover:bg-neutral-50 dark:hover:bg-neutral-800 hover:border-neutral-300 dark:hover:border-neutral-700 text-neutral-700 dark:text-neutral-300 transition-colors duration-150 cursor-pointer shrink-0 shadow-2xs"
            title="Next day"
          >
            <ChevronRight className="w-4 h-4" />
          </button>
        </div>

        {selectedDate !== todayStr && (
          <button
            type="button"
            onClick={onSelectToday}
            className="h-8 px-2.5 rounded-md text-xs font-medium border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 hover:bg-neutral-50 dark:hover:bg-neutral-800 hover:border-neutral-300 dark:hover:border-neutral-700 text-neutral-800 dark:text-neutral-200 transition-colors duration-150 cursor-pointer shrink-0 inline-flex items-center gap-1 shadow-2xs"
          >
            <span>Back to today</span>
          </button>
        )}

        <button
          type="button"
          onClick={() =>
            triggerAiPrompt(
              `Generate a thoughtful reflection and summary of my activities, mood, habits, and highlights for ${selectedDate}.`
            )
          }
          className="h-8 px-2.5 rounded-md text-xs font-medium border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 hover:bg-neutral-50 dark:hover:bg-neutral-800 hover:border-neutral-300 dark:hover:border-neutral-700 text-neutral-800 dark:text-neutral-200 transition-colors duration-150 cursor-pointer shrink-0 inline-flex items-center gap-1.5 shadow-2xs"
          title="Ask AI Brain to reflect on this day"
        >
          <Sparkles className="w-3.5 h-3.5 text-neutral-500 dark:text-neutral-400" />
          <span>AI Reflection</span>
        </button>
      </div>
    </div>
  );
};
