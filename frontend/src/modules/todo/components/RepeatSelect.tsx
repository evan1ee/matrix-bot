import React from 'react';
import { RECURRENCE_OPTIONS } from '../types';

export interface RepeatSelectProps {
  value: string;
  onChange: (value: string) => void;
  /** A repeat counts forward from the due date, so it needs one. */
  hasDueDate: boolean;
}

export const RepeatSelect: React.FC<RepeatSelectProps> = ({ value, onChange, hasDueDate }) => (
  <label className="flex flex-col gap-1">
    <span className="text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans">Repeat</span>
    <select
      value={hasDueDate ? value : 'none'}
      disabled={!hasDueDate}
      onChange={(e) => onChange(e.target.value)}
      title={hasDueDate ? undefined : 'Set a due date to repeat this task'}
      className="h-9 px-2 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs font-mono text-neutral-900 dark:text-neutral-100 focus:outline-none focus-visible:border-neutral-500 transition-colors duration-150 disabled:opacity-50 cursor-pointer disabled:cursor-not-allowed"
    >
      {RECURRENCE_OPTIONS.map((r) => (
        <option key={r} value={r}>
          {r === 'none' ? 'Does not repeat' : r}
        </option>
      ))}
    </select>
  </label>
);
