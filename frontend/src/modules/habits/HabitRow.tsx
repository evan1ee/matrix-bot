import React from 'react';
import { CheckCircle2, Circle, Flame, Pencil, Trash2 } from 'lucide-react';
import type { HabitWithStatus } from './types';

interface HabitRowProps {
  habit: HabitWithStatus;
  onToggle: (id: number) => void;
  onEdit: (habit: HabitWithStatus) => void;
  onDelete: (id: number) => void;
  isToggling?: boolean;
}

export const HabitRow: React.FC<HabitRowProps> = ({
  habit,
  onToggle,
  onEdit,
  onDelete,
  isToggling,
}) => {
  return (
    <div
      data-testid={`habit-row-${habit.id}`}
      className="group flex items-center justify-between p-3 rounded-md transition-colors hover:bg-neutral-100/60 dark:hover:bg-neutral-900/50"
    >
      <div className="flex items-center gap-3 min-w-0 flex-1">
        {/* Toggle Button */}
        <button
          type="button"
          onClick={() => onToggle(habit.id)}
          disabled={isToggling}
          aria-label={habit.completed_today ? `Uncheck ${habit.name}` : `Check off ${habit.name}`}
          className="shrink-0 text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 transition disabled:opacity-50 cursor-pointer"
        >
          {habit.completed_today ? (
            <CheckCircle2 className="w-5 h-5 text-neutral-900 dark:text-neutral-100 fill-neutral-900 dark:fill-neutral-100 text-white dark:text-black" />
          ) : (
            <Circle className="w-5 h-5" />
          )}
        </button>

        {/* Habit Details */}
        <div className="min-w-0 flex-1 pr-2">
          <div className="flex items-center gap-2 flex-wrap">
            <span
              className={`text-sm font-medium font-sans truncate ${
                habit.completed_today
                  ? 'line-through text-neutral-400 dark:text-neutral-500'
                  : 'text-neutral-900 dark:text-neutral-100'
              }`}
            >
              {habit.name}
            </span>
            <span className="text-[10px] font-mono px-1.5 py-0.5 rounded bg-neutral-200/60 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400 uppercase tracking-wider">
              {habit.category}
            </span>
          </div>

          {habit.description && (
            <p className="text-xs text-neutral-500 truncate mt-0.5 font-sans">
              {habit.description}
            </p>
          )}
        </div>
      </div>

      {/* Right-side metadata & actions */}
      <div className="flex items-center gap-4 shrink-0">
        {/* Trailing 7-day dots */}
        <div className="hidden sm:flex items-center gap-1" title="Last 7 days completion">
          {habit.last_7_days.map((day) => (
            <div
              key={day.date}
              title={`${day.date}: ${day.completed ? 'Completed' : 'Missed'}`}
              className={`w-2 h-2 rounded-xs ${
                day.completed
                  ? 'bg-neutral-900 dark:bg-neutral-100'
                  : 'bg-neutral-200 dark:bg-neutral-800'
              }`}
            />
          ))}
        </div>

        {/* Streak Badge */}
        <div
          className={`flex items-center gap-1 font-mono text-xs tabular-nums px-2 py-0.5 rounded ${
            habit.current_streak > 0
              ? 'bg-neutral-200/70 dark:bg-neutral-800/80 text-neutral-900 dark:text-neutral-100 font-medium'
              : 'text-neutral-400'
          }`}
          title={`Current streak: ${habit.current_streak} days, Best: ${habit.longest_streak} days`}
        >
          <Flame className={`w-3.5 h-3.5 ${habit.current_streak > 0 ? 'text-amber-500' : 'text-neutral-400'}`} />
          <span>{habit.current_streak}d</span>
        </div>

        {/* Action Buttons */}
        <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
          <button
            type="button"
            onClick={() => onEdit(habit)}
            aria-label={`Edit ${habit.name}`}
            className="p-1.5 rounded text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200 hover:bg-neutral-200 dark:hover:bg-neutral-800 transition cursor-pointer"
          >
            <Pencil className="w-3.5 h-3.5" />
          </button>
          <button
            type="button"
            onClick={() => onDelete(habit.id)}
            aria-label={`Delete ${habit.name}`}
            className="p-1.5 rounded text-neutral-400 hover:text-red-600 dark:hover:text-red-400 hover:bg-neutral-200 dark:hover:bg-neutral-800 transition cursor-pointer"
          >
            <Trash2 className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>
    </div>
  );
};
