import React from 'react';
import { CheckSquare, Plus, AlertCircle } from 'lucide-react';
import { CustomSelect } from '../../../components/CustomSelect';
import { DateTimePicker } from '../../../components/DateTimePicker';
import { nowInZone, useSettings } from '../../../core/settings';
import { JoinedRoomInfo, LEAD_OPTIONS } from '../types';

export interface CreateTodoCardProps {
  text: string;
  setText: (val: string) => void;
  priority: string;
  setPriority: (val: string) => void;
  dueAt: string;
  setDueAt: (val: string) => void;
  leadMinutes: string;
  setLeadMinutes: (val: string) => void;
  remindRoomId: string;
  setRemindRoomId: (val: string) => void;
  rooms?: JoinedRoomInfo[];
  isPending: boolean;
  errorMsg: string | null;
  onSubmit: (e: React.FormEvent) => void;
  textInputRef: React.RefObject<HTMLInputElement | null>;
}

export const CreateTodoCard: React.FC<CreateTodoCardProps> = ({
  text,
  setText,
  priority,
  setPriority,
  dueAt,
  setDueAt,
  leadMinutes,
  setLeadMinutes,
  remindRoomId,
  setRemindRoomId,
  rooms,
  isPending,
  errorMsg,
  onSubmit,
  textInputRef,
}) => {
  const { timezone } = useSettings();
  const wantsReminder = leadMinutes !== '';
  const noRoomsAvailable = wantsReminder && (rooms?.length ?? 0) === 0;

  return (
    <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4">
      <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3">
        <div className="flex items-center gap-2">
          <CheckSquare className="w-4 h-4 text-neutral-600 dark:text-neutral-300" />
          <h3 className="text-sm font-medium text-neutral-900 dark:text-neutral-200">Create New Task</h3>
        </div>
        <span className="text-xs font-mono text-neutral-500">Max 500 chars</span>
      </div>

      {errorMsg && (
        <div className="flex items-center gap-2 p-3 rounded bg-red-100 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 text-red-800 dark:text-red-300 text-xs font-mono">
          <AlertCircle className="w-4 h-4 text-red-500 dark:text-red-400 shrink-0" />
          <span>{errorMsg}</span>
        </div>
      )}

      <form onSubmit={onSubmit} className="space-y-3">
        <div className="grid grid-cols-1 sm:grid-cols-4 gap-3">
          <input
            ref={textInputRef}
            type="text"
            placeholder="What needs to be done?"
            value={text}
            onChange={(e) => setText(e.target.value)}
            className="sm:col-span-2 h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 focus:outline-none focus:border-neutral-500 font-sans"
          />

          <CustomSelect
            value={priority}
            onChange={(val) => setPriority(val)}
            mono
            options={[
              { value: 'low', label: 'Low Priority' },
              { value: 'medium', label: 'Medium Priority' },
              { value: 'high', label: 'High Priority' },
            ]}
          />

          <button
            type="submit"
            disabled={isPending}
            className="h-9 inline-flex items-center justify-center gap-2 px-4 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50 cursor-pointer"
          >
            <Plus className="w-4 h-4" />
            <span>{isPending ? 'Adding...' : 'Add Task'}</span>
          </button>
        </div>

        <div className="grid grid-cols-1 sm:grid-cols-4 gap-3">
          <label className="sm:col-span-2 flex flex-col gap-1">
            <span className="text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans">
              Due date (optional)
            </span>
            <DateTimePicker
              value={dueAt}
              onChange={setDueAt}
              placeholder="No due date"
              now={nowInZone(timezone)}
            />
          </label>

          <label className="flex flex-col gap-1">
            <span className="text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans">Remind me</span>
            <CustomSelect
              value={leadMinutes}
              onChange={setLeadMinutes}
              mono
              options={LEAD_OPTIONS}
            />
          </label>

          {wantsReminder && (
            <label className="flex flex-col gap-1">
              <span className="text-xs font-medium text-neutral-600 dark:text-neutral-400 font-sans">Send to room</span>
              <CustomSelect
                value={remindRoomId}
                onChange={setRemindRoomId}
                mono
                options={[
                  { value: '', label: 'Select a room…' },
                  ...(rooms ?? []).map((r) => ({ value: r.room_id, label: r.name })),
                ]}
              />
            </label>
          )}
        </div>

        {noRoomsAvailable && (
          <p className="text-xs font-mono text-amber-700 dark:text-amber-400">
            No Matrix rooms available. Reminders are delivered to a room you and the bot both
            belong to — sign in with your Matrix account to pick one.
          </p>
        )}
      </form>
    </div>
  );
};
