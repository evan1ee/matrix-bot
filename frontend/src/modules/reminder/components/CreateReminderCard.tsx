import React from 'react';
import { Clock, Plus, AlertCircle } from 'lucide-react';
import { CustomSelect } from '../../../components/CustomSelect';
import type { JoinedRoomInfo, DurationUnit } from '../types';

export interface CreateReminderCardProps {
  message: string;
  setMessage: (val: string) => void;
  durationValue: number;
  setDurationValue: (val: number) => void;
  durationUnit: DurationUnit;
  setDurationUnit: (val: DurationUnit) => void;
  hour12: string;
  setHour12: (val: string) => void;
  minute: string;
  setMinute: (val: string) => void;
  amPm: 'AM' | 'PM';
  setAmPm: (val: 'AM' | 'PM') => void;
  recurrence: string;
  setRecurrence: (val: string) => void;
  roomId: string;
  setRoomId: (val: string) => void;
  roomsData?: JoinedRoomInfo[];
  errorMsg: string | null;
  isPending: boolean;
  onSubmit: (e: React.FormEvent) => void;
  messageInputRef: React.RefObject<HTMLInputElement | null>;
}

export const CreateReminderCard: React.FC<CreateReminderCardProps> = ({
  message,
  setMessage,
  durationValue,
  setDurationValue,
  durationUnit,
  setDurationUnit,
  hour12,
  setHour12,
  minute,
  setMinute,
  amPm,
  setAmPm,
  recurrence,
  setRecurrence,
  roomId,
  setRoomId,
  roomsData,
  errorMsg,
  isPending,
  onSubmit,
  messageInputRef,
}) => {
  return (
    <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4">
      <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3">
        <div className="flex items-center gap-2">
          <Clock className="w-4 h-4 text-neutral-600 dark:text-neutral-300" />
          <h3 className="text-sm font-medium text-neutral-900 dark:text-neutral-200">Schedule reminder</h3>
        </div>
        <span className="text-xs font-mono text-neutral-500">Scheduled execution</span>
      </div>

      {errorMsg && (
        <div className="flex items-center gap-2 p-3 rounded bg-red-100 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 text-red-800 dark:text-red-300 text-xs font-mono">
          <AlertCircle className="w-4 h-4 text-red-500 dark:text-red-400 shrink-0" />
          <span>{errorMsg}</span>
        </div>
      )}

      <form onSubmit={onSubmit} className="space-y-3">
        <div className="grid grid-cols-1 sm:grid-cols-6 gap-3">
          <input
            ref={messageInputRef}
            type="text"
            placeholder="Reminder message..."
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            className="sm:col-span-2 h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 focus:outline-none focus:border-neutral-500 font-sans"
          />

          {recurrence === 'none' ? (
            <div className="h-9 flex items-center gap-1 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 shadow-xs">
              <span className="text-xs font-mono text-neutral-500">In</span>
              <input
                type="number"
                min="1"
                value={durationValue}
                onChange={(e) => setDurationValue(parseInt(e.target.value) || 1)}
                className="w-12 bg-transparent text-xs text-neutral-900 dark:text-neutral-100 font-mono focus:outline-none text-center"
              />
              <CustomSelect
                value={durationUnit}
                onChange={(val) => setDurationUnit(val as DurationUnit)}
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
          ) : (
            <div className="h-9 flex items-center gap-1 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 shadow-xs font-mono text-xs">
              <span className="text-neutral-500">At</span>
              <CustomSelect
                value={hour12}
                onChange={(val) => setHour12(val)}
                variant="inline"
                mono
                options={Array.from({ length: 12 }, (_, i) => i + 1).map((h) => {
                  const val = h < 10 ? `0${h}` : `${h}`;
                  return { value: val, label: val };
                })}
              />
              <span className="text-neutral-400 font-mono">:</span>
              <CustomSelect
                value={minute}
                onChange={(val) => setMinute(val)}
                variant="inline"
                mono
                options={['00', '05', '10', '15', '20', '25', '30', '35', '40', '45', '50', '55'].map((m) => ({
                  value: m,
                  label: m,
                }))}
              />
              <button
                type="button"
                onClick={() => setAmPm(amPm === 'AM' ? 'PM' : 'AM')}
                className="ml-1 px-1.5 py-0.5 rounded bg-neutral-100 dark:bg-neutral-800 text-[10px] uppercase font-mono text-neutral-700 dark:text-neutral-300 hover:bg-neutral-200 dark:hover:bg-neutral-700 transition cursor-pointer"
              >
                {amPm}
              </button>
            </div>
          )}

          <CustomSelect
            value={recurrence}
            onChange={(val) => setRecurrence(val)}
            mono
            options={[
              { value: 'none', label: 'One-time' },
              { value: 'daily', label: 'Daily' },
              { value: 'weekly', label: 'Weekly' },
              { value: 'monthly', label: 'Monthly' },
              { value: 'yearly', label: 'Yearly' },
            ]}
          />

          <CustomSelect
            value={roomId}
            onChange={(val) => setRoomId(val)}
            options={
              roomsData && roomsData.length > 0
                ? roomsData.map((r) => ({ value: r.room_id, label: r.name }))
                : [{ value: '', label: 'Loading rooms...' }]
            }
          />

          <button
            type="submit"
            disabled={isPending}
            className="h-9 inline-flex items-center justify-center gap-2 px-4 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50 cursor-pointer"
          >
            <Plus className="w-4 h-4" />
            <span>{isPending ? 'Scheduling...' : 'Set reminder'}</span>
          </button>
        </div>
      </form>
    </div>
  );
};
