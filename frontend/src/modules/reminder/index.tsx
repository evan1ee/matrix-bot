import React, { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import { FrontendModule, SystemEvent } from '../../core/types';
import { Clock, Plus, Trash2, BellRing, Repeat, AlertCircle, Pencil } from 'lucide-react';
import { CustomSelect } from '../../components/CustomSelect';
import { formatTimeInZone, nowInZone, useSettings } from '../../core/settings';

export interface ReminderItem {
  id: number;
  room_id: string;
  sender_mxid: string;
  sender_name: string;
  message: string;
  trigger_at: string;
  recurrence: string;
  status: string;
  retry_count: number;
  created_at?: string | null;
}

export interface ReminderListResponse {
  items: ReminderItem[];
  page: number;
  limit: number;
  total: number;
}

export interface JoinedRoomInfo {
  room_id: string;
  name: string;
}

/** Duration units, and how many seconds one of each is worth. */
const UNIT_SECONDS = {
  seconds: 1,
  minutes: 60,
  hours: 3600,
  days: 86400,
} as const;

type DurationUnit = keyof typeof UNIT_SECONDS;

/**
 * Expresses a delay in the largest unit worth at least one whole step.
 *
 * The API takes a delay from now, so the edit form has to turn the stored
 * absolute `trigger_at` back into one. The value is *rounded* rather than
 * required to divide exactly: a reminder set for "in 2 hours" and opened a
 * minute later has 7121 seconds left, and "In 2 Hours" is what the user meant,
 * where "In 7121 Secs" is technically true and unreadable. The cost is that
 * saving an untouched form can shift the fire time by less than one unit —
 * acceptable when re-deriving from "now" already moves it anyway.
 */
function secondsToDuration(totalSeconds: number): { value: number; unit: DurationUnit } {
  const units: DurationUnit[] = ['days', 'hours', 'minutes', 'seconds'];
  for (const unit of units) {
    const size = UNIT_SECONDS[unit];
    if (totalSeconds >= size) {
      return { value: Math.max(1, Math.round(totalSeconds / size)), unit };
    }
  }
  return { value: Math.max(1, totalSeconds), unit: 'seconds' };
}

/** One pending reminder: a read-only row that swaps into an edit form. */
const ReminderRow: React.FC<{
  item: ReminderItem;
  timezone: string;
  roomLabel: (roomId: string) => string;
  timeLeft: string;
  onDelete: (id: number) => void;
}> = ({ item, timezone, roomLabel, timeLeft, onDelete }) => {
  const queryClient = useQueryClient();
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
            className="sm:col-span-2 px-3 py-2 rounded bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-sans"
          />

          <div className="flex items-center gap-1 px-3 py-2 rounded-lg bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800">
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
            className="inline-flex items-center gap-2 px-3 py-1.5 rounded bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50"
          >
            {editMutation.isPending ? 'Saving...' : 'Save'}
          </button>
          <button
            type="button"
            onClick={() => setEditing(false)}
            className="px-3 py-1.5 rounded border border-neutral-300 dark:border-neutral-700 text-xs font-medium text-neutral-700 dark:text-neutral-300 hover:bg-neutral-200 dark:hover:bg-neutral-800 transition"
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
            <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] uppercase font-mono bg-blue-100 dark:bg-blue-950/40 text-blue-800 dark:text-blue-300 border border-blue-300 dark:border-blue-900/50">
              <Repeat className="w-3 h-3" />
              {item.recurrence}
            </span>
          )}
        </div>
        <div className="flex items-center gap-2 text-xs font-mono text-neutral-500">
          <span>Target: {formatTimeInZone(item.trigger_at, timezone)}</span>
          <span>•</span>
          <span>by {item.sender_name}</span>
          <span>•</span>
          <span className="truncate max-w-[180px]">Room: {roomLabel(item.room_id)}</span>
        </div>
      </div>

      <div className="flex items-center gap-3 shrink-0">
        <span className="px-2.5 py-1 rounded bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-xs font-mono text-amber-600 dark:text-amber-300 animate-pulse">
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

export const ReminderMainView: React.FC<{ autoFocus?: boolean }> = ({ autoFocus }) => {
  const { timezone } = useSettings();
  const queryClient = useQueryClient();
  const messageInputRef = React.useRef<HTMLInputElement>(null);
  const [message, setMessage] = useState('');
  const [durationValue, setDurationValue] = useState<number>(5);
  const [durationUnit, setDurationUnit] = useState<'seconds' | 'minutes' | 'hours' | 'days'>('minutes');
  
  // Custom Time Picker State
  const [hour12, setHour12] = useState('09');
  const [minute, setMinute] = useState('30');
  const [amPm, setAmPm] = useState<'AM' | 'PM'>('AM');

  const [recurrence, setRecurrence] = useState('none');
  const [roomId, setRoomId] = useState('');
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [now, setNow] = useState<number>(Date.now());

  React.useEffect(() => {
    const timer = setInterval(() => {
      setNow(Date.now());
    }, 1000);
    return () => clearInterval(timer);
  }, []);

  React.useEffect(() => {
    if (autoFocus) messageInputRef.current?.focus();
  }, [autoFocus]);

  const { data, isLoading } = useQuery<ReminderListResponse>({
    queryKey: ['reminders'],
    queryFn: () => fetchApi<ReminderListResponse>('/reminders?limit=100'),
  });

  const { data: roomsData } = useQuery<JoinedRoomInfo[]>({
    queryKey: ['rooms'],
    queryFn: () => fetchApi<JoinedRoomInfo[]>('/rooms'),
  });

  React.useEffect(() => {
    if (roomsData && roomsData.length > 0 && (!roomId || roomId === 'web')) {
      setRoomId(roomsData[0].room_id);
    }
  }, [roomsData]);

  const createMutation = useMutation({
    mutationFn: (newReminder: { room_id: string; message: string; seconds: number; recurrence: string }) =>
      fetchApi<ReminderItem>('/reminders', {
        method: 'POST',
        body: JSON.stringify(newReminder),
      }),
    onSuccess: () => {
      setMessage('');
      setErrorMsg(null);
      queryClient.invalidateQueries({ queryKey: ['reminders'] });
    },
    onError: (err: Error) => {
      setErrorMsg(err.message);
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: number) =>
      fetchApi(`/reminders/${id}`, {
        method: 'DELETE',
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['reminders'] });
    },
  });

  const getCalculatedSeconds = (): number => {
    switch (durationUnit) {
      case 'seconds':
        return durationValue;
      case 'minutes':
        return durationValue * 60;
      case 'hours':
        return durationValue * 3600;
      case 'days':
        return durationValue * 86400;
      default:
        return durationValue * 60;
    }
  };

  const calculateSecondsForRecurringTime = (): number => {
    let targetHours = parseInt(hour12, 10) || 9;
    const targetMinutes = parseInt(minute, 10) || 0;

    if (amPm === 'PM' && targetHours < 12) {
      targetHours += 12;
    } else if (amPm === 'AM' && targetHours === 12) {
      targetHours = 0;
    }

    // "Daily at 8am" means 8am where the user is, not where their browser is.
    // Both clocks are the same zone's wall clock, so their difference is the
    // real interval to wait.
    const currDate = nowInZone(timezone);
    const target = nowInZone(timezone);
    target.setHours(targetHours, targetMinutes, 0, 0);

    if (target.getTime() <= currDate.getTime()) {
      target.setDate(target.getDate() + 1);
    }

    return Math.max(1, Math.floor((target.getTime() - currDate.getTime()) / 1000));
  };

  const handleCreate = (e: React.FormEvent) => {
    e.preventDefault();
    if (!message.trim()) return;

    const computedSecs =
      recurrence === 'none'
        ? getCalculatedSeconds()
        : calculateSecondsForRecurringTime();

    if (computedSecs <= 0) {
      setErrorMsg('Duration must be greater than 0.');
      return;
    }
    if (message.length > 500) {
      setErrorMsg('Message exceeds 500 character limit.');
      return;
    }
    const targetRoom = roomId || (roomsData && roomsData.length > 0 ? roomsData[0].room_id : 'web');
    createMutation.mutate({ room_id: targetRoom, message: message.trim(), seconds: computedSecs, recurrence });
  };

  const calculateTimeLeft = (triggerAtStr: string) => {
    const diffMs = new Date(triggerAtStr).getTime() - now;
    if (diffMs <= 0) return 'Triggering...';
    const totalSecs = Math.floor(diffMs / 1000);
    const mins = Math.floor(totalSecs / 60);
    const secs = totalSecs % 60;
    if (mins >= 60) {
      const hours = Math.floor(mins / 60);
      return `in ${hours}h ${mins % 60}m`;
    }
    if (mins > 0) return `in ${mins}m ${secs}s`;
    return `in ${secs}s`;
  };

  const getRoomDisplayName = (id: string) => {
    if (!roomsData) return id;
    const match = roomsData.find((r) => r.room_id === id);
    return match ? match.name : id;
  };

  return (
    <div className="space-y-6">
      {/* Create Reminder Form */}
      <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4">
        <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3">
          <div className="flex items-center gap-2">
            <Clock className="w-4 h-4 text-neutral-600 dark:text-neutral-300" />
            <h3 className="text-sm font-medium text-neutral-900 dark:text-neutral-200">Schedule Reminder</h3>
          </div>
          <span className="text-xs font-mono text-neutral-500">Scheduled Execution</span>
        </div>

        {errorMsg && (
          <div className="flex items-center gap-2 p-3 rounded bg-red-100 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 text-red-800 dark:text-red-300 text-xs font-mono">
            <AlertCircle className="w-4 h-4 text-red-500 dark:text-red-400 shrink-0" />
            <span>{errorMsg}</span>
          </div>
        )}

        <form onSubmit={handleCreate} className="space-y-3">
          <div className="grid grid-cols-1 sm:grid-cols-6 gap-3">
            <input
              ref={messageInputRef}
              type="text"
              placeholder="Reminder message..."
              value={message}
              onChange={(e) => setMessage(e.target.value)}
              className="sm:col-span-2 px-3 py-2 rounded bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 focus:outline-none focus:border-neutral-500 font-sans"
            />

            {recurrence === 'none' ? (
              <div className="flex items-center gap-1 px-3 py-2 rounded-lg bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 shadow-sm">
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
                  onChange={(val) => setDurationUnit(val as any)}
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
              <div className="flex items-center gap-1 px-3 py-2 rounded-lg bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 shadow-sm font-mono text-xs">
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
              disabled={createMutation.isPending}
              className="inline-flex items-center justify-center gap-2 px-4 py-2 min-h-11 rounded bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50"
            >
              <Plus className="w-4 h-4" />
              <span>{createMutation.isPending ? 'Scheduling...' : 'Set Reminder'}</span>
            </button>
          </div>
        </form>
      </div>

      {/* Reminder List */}
      <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4">
        <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3">
          <div className="flex items-center gap-2">
            <BellRing className="w-4 h-4 text-neutral-600 dark:text-neutral-300" />
            <h3 className="text-sm font-medium text-neutral-900 dark:text-neutral-200">Pending Reminders</h3>
          </div>
          <span className="text-xs font-mono text-neutral-500">
            {data?.items.length || 0} active
          </span>
        </div>

        {isLoading ? (
          <div className="py-8 text-center text-xs font-mono text-neutral-500">Loading reminders...</div>
        ) : !data || data.items.length === 0 ? (
          <div className="py-8 text-center text-xs font-mono text-neutral-500 border border-dashed border-neutral-300 dark:border-neutral-800 rounded">
            No pending reminders. Set one above!
          </div>
        ) : (
          <div className="divide-y divide-neutral-200 dark:divide-neutral-850">
            {data.items.map((item) => (
              <ReminderRow
                key={item.id}
                item={item}
                timezone={timezone}
                roomLabel={getRoomDisplayName}
                timeLeft={calculateTimeLeft(item.trigger_at)}
                onDelete={(id) => deleteMutation.mutate(id)}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
};

export const reminderModule: FrontendModule = {
  id: 'reminder',
  name: 'Reminders',
  description: 'Scheduled execution and recurring reminders',
  navTab: { label: 'Reminders' },
  MainView: ReminderMainView,
  handleSseEvent: (event, queryClient) => {
    if (
      event.type === 'ReminderCreated' ||
      event.type === 'ReminderUpdated' ||
      event.type === 'ReminderTriggered' ||
      event.type === 'ReminderCancelled'
    ) {
      queryClient.invalidateQueries({ queryKey: ['reminders'] });
    }
  },
};
