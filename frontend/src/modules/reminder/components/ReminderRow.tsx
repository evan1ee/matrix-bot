import React, { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../../core/apiClient';
import { Trash2, Repeat, AlertCircle, Pencil } from 'lucide-react';
import { CustomSelect } from '../../../components/CustomSelect';
import { formatTimeInZone } from '../../../core/settings';
import { useSession } from '../../../core/session';
import type { ReminderItem, DurationUnit } from '../types';
import { UNIT_SECONDS, secondsToDuration, formatSenderDisplay } from '../types';

export interface ReminderRowProps {
  item: ReminderItem;
  timezone: string;
  roomLabel: (roomId: string) => string;
  timeLeft: string;
  onDelete: (id: number) => void;
}

/** One pending reminder: a read-only row that swaps into an edit form. */
export const ReminderRow: React.FC<ReminderRowProps> = ({
  item,
  timezone,
  roomLabel,
  timeLeft,
  onDelete,
}) => {
  const queryClient = useQueryClient();
  const { user } = useSession();
  const [editing, setEditing] = useState(false);
  const [draftMessage, setDraftMessage] = useState(item.message);
  const [draftValue, setDraftValue] = useState(1);
  const [draftUnit, setDraftUnit] = useState<DurationUnit>('hours');
  const [draftRecurrence, setDraftRecurrence] = useState(item.recurrence);
  const [rowError, setRowError] = useState<string | null>(null);

  const startEditing = () => {
    const remaining = Math.round((new Date(item.trigger_at).getTime() - Date.now()) / 1000);
    const { value, unit } = secondsToDuration(Math.max(1, remaining));
    setDraftMessage(item.message);
    setDraftValue(value);
    setDraftUnit(unit);
    setDraftRecurrence(item.recurrence);
    setRowError(null);
    setEditing(true);
  };

  const editMutation = useMutation({
    mutationFn: (body: { message: string; seconds: number; recurrence: string }) =>
      fetchApi<ReminderItem>(`/reminders/${item.id}`, {
        method: 'PATCH',
        body: JSON.stringify(body),
      }),
    onSuccess: () => {
      setEditing(false);
      setRowError(null);
      queryClient.invalidateQueries({ queryKey: ['reminders'] });
    },
    onError: (err: Error) => setRowError(err.message),
  });

  const save = (e: React.FormEvent) => {
    e.preventDefault();
    if (!draftMessage.trim()) {
      setRowError('Message required');
      return;
    }
    editMutation.mutate({
      message: draftMessage.trim(),
      seconds: draftValue * UNIT_SECONDS[draftUnit],
      recurrence: draftRecurrence,
    });
  };

  if (editing) {
    return (
      <form onSubmit={save} className="py-3 px-2 space-y-3 bg-neutral-100/60 dark:bg-neutral-900/30 rounded">
        {rowError && (
          <div className="flex items-center gap-2 text-xs font-mono text-red-600 dark:text-red-400">
            <AlertCircle className="w-3.5 h-3.5 shrink-0" />
            <span>{rowError}</span>
          </div>
        )}

        <div className="grid grid-cols-1 sm:grid-cols-4 gap-3">
          <input
            type="text"
            value={draftMessage}
            onChange={(e) => setDraftMessage(e.target.value)}
            className="sm:col-span-2 h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-sans"
          />

          <div className="h-9 flex items-center gap-1 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800">
            <span className="text-xs font-mono text-neutral-500">In</span>
            <input
              type="number"
              min="1"
              value={draftValue}
              onChange={(e) => setDraftValue(parseInt(e.target.value) || 1)}
              className="w-12 bg-transparent text-xs text-neutral-900 dark:text-neutral-100 font-mono focus:outline-none text-center"
            />
            <CustomSelect
              value={draftUnit}
              onChange={(val) => setDraftUnit(val as DurationUnit)}
              variant="inline"
              mono
              options={[
                { value: 'seconds', label: 'Secs' },
                { value: 'minutes', label: 'Mins' },
                { value: 'hours', label: 'Hours' },
                { value: 'days', label: 'Days' },
              ]}
            />
          </div>

          <CustomSelect
            value={draftRecurrence}
            onChange={setDraftRecurrence}
            mono
            options={[
              { value: 'none', label: 'One-time' },
              { value: 'daily', label: 'Daily' },
              { value: 'weekly', label: 'Weekly' },
              { value: 'monthly', label: 'Monthly' },
              { value: 'yearly', label: 'Yearly' },
            ]}
          />
        </div>

        <div className="flex items-center gap-2">
          <button
            type="submit"
            disabled={editMutation.isPending}
            className="h-9 inline-flex items-center gap-2 px-3 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50 cursor-pointer"
          >
            {editMutation.isPending ? 'Saving...' : 'Save'}
          </button>
          <button
            type="button"
            onClick={() => setEditing(false)}
            className="h-9 px-3 rounded-md border border-neutral-300 dark:border-neutral-700 text-xs font-medium text-neutral-700 dark:text-neutral-300 hover:bg-neutral-200 dark:hover:bg-neutral-800 transition cursor-pointer"
          >
            Cancel
          </button>
        </div>
      </form>
    );
  }

  return (
    <div className="py-3 flex items-center justify-between gap-4 transition hover:bg-neutral-100 dark:hover:bg-neutral-900/30 px-2 rounded">
      <div className="space-y-1 min-w-0">
        <div className="flex items-center gap-2">
          <p className="text-sm font-medium text-neutral-900 dark:text-neutral-100">{item.message}</p>
          {item.recurrence !== 'none' && (
            <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-mono font-medium bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400 border border-neutral-200 dark:border-neutral-700">
              <Repeat className="w-3 h-3" />
              {item.recurrence}
            </span>
          )}
        </div>
        <div className="flex items-center gap-2 text-xs font-mono text-neutral-500">
          <span>Target: {formatTimeInZone(item.trigger_at, timezone)}</span>
          <span>•</span>
          <span>by {formatSenderDisplay(item.sender_name, user?.display_name || user?.username)}</span>
          <span>•</span>
          <span className="truncate max-w-[180px]">Room: {roomLabel(item.room_id)}</span>
        </div>
      </div>

      <div className="flex items-center gap-3 shrink-0">
        <span className="px-2.5 py-1 rounded bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-xs font-mono text-amber-600 dark:text-amber-300">
          {timeLeft}
        </span>
        <button
          onClick={startEditing}
          className="p-1.5 rounded text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-200 dark:hover:bg-neutral-900 transition"
          title="Edit reminder"
        >
          <Pencil className="w-4 h-4" />
        </button>
        <button
          onClick={() => onDelete(item.id)}
          className="p-1.5 rounded text-neutral-400 hover:text-rose-600 dark:hover:text-rose-400 hover:bg-neutral-200 dark:hover:bg-neutral-900 transition"
          title="Cancel reminder"
        >
          <Trash2 className="w-4 h-4" />
        </button>
      </div>
    </div>
  );
};
