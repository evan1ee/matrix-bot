import React, { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import { FrontendModule, SystemEvent } from '../../core/types';
import { Clock, Plus, Trash2, BellRing, Repeat, AlertCircle } from 'lucide-react';

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
}

export const ReminderMainView: React.FC = () => {
  const queryClient = useQueryClient();
  const [message, setMessage] = useState('');
  const [seconds, setSeconds] = useState(60);
  const [recurrence, setRecurrence] = useState('none');
  const [roomId, setRoomId] = useState('!global:matrix.org');
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const { data, isLoading } = useQuery<ReminderListResponse>({
    queryKey: ['reminders'],
    queryFn: () => fetchApi<ReminderListResponse>('/reminders?limit=100'),
  });

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

  const handleCreate = (e: React.FormEvent) => {
    e.preventDefault();
    if (!message.trim()) return;
    if (seconds <= 0) {
      setErrorMsg('Duration must be greater than 0 seconds.');
      return;
    }
    if (message.length > 500) {
      setErrorMsg('Message exceeds 500 character limit.');
      return;
    }
    createMutation.mutate({ room_id: roomId, message: message.trim(), seconds, recurrence });
  };

  const calculateTimeLeft = (triggerAtStr: string) => {
    const diffMs = new Date(triggerAtStr).getTime() - new Date().getTime();
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
          <div className="grid grid-cols-1 sm:grid-cols-5 gap-3">
            <input
              type="text"
              placeholder="Reminder message..."
              value={message}
              onChange={(e) => setMessage(e.target.value)}
              className="sm:col-span-2 px-3 py-2 rounded bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 focus:outline-none focus:border-neutral-500 font-sans"
            />

            <div className="flex items-center gap-1.5 px-3 py-2 rounded bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800">
              <span className="text-xs font-mono text-neutral-500">In</span>
              <input
                type="number"
                min="1"
                value={seconds}
                onChange={(e) => setSeconds(parseInt(e.target.value) || 0)}
                className="w-16 bg-transparent text-xs text-neutral-900 dark:text-neutral-100 font-mono focus:outline-none"
              />
              <span className="text-xs font-mono text-neutral-500">secs</span>
            </div>

            <select
              value={recurrence}
              onChange={(e) => setRecurrence(e.target.value)}
              className="px-3 py-2 rounded bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-800 dark:text-neutral-200 font-mono focus:outline-none focus:border-neutral-500"
            >
              <option value="none">One-time</option>
              <option value="daily">Daily</option>
              <option value="weekly">Weekly</option>
            </select>

            <button
              type="submit"
              disabled={createMutation.isPending}
              className="inline-flex items-center justify-center gap-2 px-4 py-2 rounded bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50"
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
          <div className="py-8 text-center text-xs font-mono text-neutral-500 border border-dashed border-neutral-300 dark:border-neutral-850 rounded">
            No pending reminders. Set one above!
          </div>
        ) : (
          <div className="divide-y divide-neutral-200 dark:divide-neutral-850">
            {data.items.map((item) => (
              <div
                key={item.id}
                className="py-3 flex items-center justify-between gap-4 transition hover:bg-neutral-100 dark:hover:bg-neutral-900/30 px-2 rounded"
              >
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
                    <span>Target: {new Date(item.trigger_at).toLocaleTimeString()}</span>
                    <span>•</span>
                    <span>by {item.sender_name}</span>
                  </div>
                </div>

                <div className="flex items-center gap-3 shrink-0">
                  <span className="px-2.5 py-1 rounded bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-xs font-mono text-amber-600 dark:text-amber-300">
                    {calculateTimeLeft(item.trigger_at)}
                  </span>
                  <button
                    onClick={() => deleteMutation.mutate(item.id)}
                    className="p-1.5 rounded text-neutral-400 hover:text-rose-600 dark:hover:text-rose-400 hover:bg-neutral-200 dark:hover:bg-neutral-900 transition"
                    title="Cancel reminder"
                  >
                    <Trash2 className="w-4 h-4" />
                  </button>
                </div>
              </div>
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
      event.type === 'ReminderTriggered' ||
      event.type === 'ReminderCancelled'
    ) {
      queryClient.invalidateQueries({ queryKey: ['reminders'] });
    }
  },
};
