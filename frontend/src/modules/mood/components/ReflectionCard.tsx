import React from 'react';
import { Trash2 } from 'lucide-react';
import type { LifeDayItem } from '../../life/types';

export interface ReflectionCardProps {
  isFormActive: boolean;
  dayLog?: LifeDayItem | null;
  selectedDate: string;
  note: string;
  onChangeNote: (val: string) => void;
  onSubmit: (e: React.FormEvent) => void;
  isSaving: boolean;
  confirmingClear: boolean;
  onSetConfirmingClear: (val: boolean) => void;
  onConfirmDelete: () => void;
  isDeleting: boolean;
}

export const ReflectionCard: React.FC<ReflectionCardProps> = ({
  isFormActive,
  dayLog,
  selectedDate,
  note,
  onChangeNote,
  onSubmit,
  isSaving,
  confirmingClear,
  onSetConfirmingClear,
  onConfirmDelete,
  isDeleting,
}) => {
  if (isFormActive) {
    return (
      <form
        onSubmit={onSubmit}
        className="border border-neutral-200 dark:border-neutral-800 rounded-lg p-5 bg-white dark:bg-neutral-900 space-y-4"
      >
        <div className="flex items-center justify-between">
          <span className="text-xs font-semibold text-neutral-900 dark:text-neutral-100 font-sans">
            Record daily reflection
          </span>
          <span className="text-[10px] font-mono text-neutral-400">
            Unrecorded
          </span>
        </div>

        <div>
          <div className="flex items-center justify-between mb-1.5">
            <label className="block text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans">
              Notes & reflection (optional)
            </label>
            <span className="text-[10px] font-mono text-neutral-400 tabular-nums">
              {note.length}/500
            </span>
          </div>
          <textarea
            value={note}
            onChange={(e) => onChangeNote(e.target.value)}
            placeholder="What happened today? Memories, achievements, or learnings..."
            maxLength={500}
            rows={8}
            className="w-full px-3.5 py-2.5 rounded-md text-xs border border-neutral-200 dark:border-neutral-700 bg-neutral-50/50 dark:bg-neutral-800/50 text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 focus:outline-hidden focus:ring-1 focus:ring-neutral-900 dark:focus:ring-neutral-100 resize-none leading-relaxed font-sans"
          />
        </div>

        {/* Submit Action */}
        <div className="pt-3 border-t border-neutral-100 dark:border-neutral-800 flex items-center justify-end">
          <button
            type="submit"
            disabled={isSaving}
            className="h-8 px-4 text-xs font-medium text-white bg-neutral-900 hover:bg-neutral-800 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-white rounded-md shadow-xs transition-colors duration-150 disabled:opacity-50 cursor-pointer inline-flex items-center gap-1.5"
          >
            {isSaving && (
              <svg
                className="animate-spin -ml-0.5 mr-1 h-3 w-3 text-white dark:text-neutral-900"
                xmlns="http://www.w3.org/2000/svg"
                fill="none"
                viewBox="0 0 24 24"
              >
                <circle
                  className="opacity-25"
                  cx="12"
                  cy="12"
                  r="10"
                  stroke="currentColor"
                  strokeWidth="4"
                />
                <path
                  className="opacity-75"
                  fill="currentColor"
                  d="M4 12a8 8 0 018-8v8H4z"
                />
              </svg>
            )}
            <span>{isSaving ? 'Saving...' : 'Save daily reflection'}</span>
          </button>
        </div>
      </form>
    );
  }

  // Sealed View
  return (
    <div className="border border-neutral-200 dark:border-neutral-800 rounded-lg p-5 bg-white dark:bg-neutral-900 space-y-4">
      <div className="flex items-center justify-between">
        <span className="text-xs font-semibold text-neutral-900 dark:text-neutral-100 font-sans">
          Sealed daily reflection
        </span>
        <span className="text-[10px] font-mono text-emerald-600 dark:text-emerald-400">
          Logged
        </span>
      </div>

      {/* Reflection Note Prose */}
      <div className="space-y-1.5">
        <div className="text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans">
          Notes & reflection
        </div>
        {dayLog?.note ? (
          <div className="p-4 rounded-lg bg-neutral-50 dark:bg-neutral-800/50 border border-neutral-200 dark:border-neutral-800 text-xs text-neutral-800 dark:text-neutral-200 whitespace-pre-wrap leading-relaxed font-sans">
            "{dayLog.note}"
          </div>
        ) : (
          <div className="p-3.5 rounded-md bg-neutral-50/50 dark:bg-neutral-800/30 border border-neutral-200/80 dark:border-neutral-800/80 text-xs text-neutral-400 dark:text-neutral-500 italic font-sans">
            No reflection notes were recorded for this day.
          </div>
        )}
      </div>

      {/* Clear Confirmation or Edit Actions */}
      {confirmingClear ? (
        <div className="p-3.5 rounded-lg border border-rose-200 dark:border-rose-900/60 bg-rose-50/50 dark:bg-rose-950/20 space-y-2.5">
          <p className="text-xs text-rose-800 dark:text-rose-200 font-medium">
            Clear reflection record for {selectedDate}?
          </p>
          <div className="flex items-center justify-end gap-2">
            <button
              type="button"
              onClick={() => onSetConfirmingClear(false)}
              className="px-2.5 py-1 text-xs text-neutral-600 dark:text-neutral-400 hover:bg-neutral-100 dark:hover:bg-neutral-800 rounded"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={onConfirmDelete}
              disabled={isDeleting}
              className="px-3 py-1 text-xs font-medium bg-rose-600 hover:bg-rose-700 text-white rounded cursor-pointer"
            >
              {isDeleting ? 'Clearing...' : 'Yes, clear record'}
            </button>
          </div>
        </div>
      ) : (
        <div className="pt-3 border-t border-neutral-100 dark:border-neutral-800 flex items-center justify-between">
          <button
            type="button"
            onClick={() => onSetConfirmingClear(true)}
            className="inline-flex items-center gap-1.5 text-xs text-rose-600 dark:text-rose-400 hover:underline cursor-pointer"
          >
            <Trash2 className="w-3.5 h-3.5" />
            <span>Clear record</span>
          </button>
        </div>
      )}
    </div>
  );
};
