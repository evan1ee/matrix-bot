import React from 'react';
import { BellRing, Sparkles } from 'lucide-react';
import { triggerAiPrompt } from '../../ai';
import { ReminderRow } from './ReminderRow';
import type { ReminderListResponse } from '../types';

export interface PendingRemindersCardProps {
  data?: ReminderListResponse;
  isLoading: boolean;
  timezone: string;
  roomLabel: (roomId: string) => string;
  calculateTimeLeft: (triggerAtStr: string) => string;
  onDelete: (id: number) => void;
}

export const PendingRemindersCard: React.FC<PendingRemindersCardProps> = ({
  data,
  isLoading,
  timezone,
  roomLabel,
  calculateTimeLeft,
  onDelete,
}) => {
  return (
    <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4">
      <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3">
        <div className="flex items-center gap-2">
          <BellRing className="w-4 h-4 text-neutral-600 dark:text-neutral-300" />
          <h3 className="text-sm font-medium text-neutral-900 dark:text-neutral-200">Pending reminders</h3>
        </div>
        <div className="flex items-center gap-3">
          <button
            type="button"
            onClick={() =>
              triggerAiPrompt(
                'Summarize my upcoming reminders and tell me if there are any conflicting or critical alerts I should prepare for.'
              )
            }
            className="inline-flex items-center gap-1.5 px-2.5 py-1 text-xs font-sans font-medium text-neutral-700 dark:text-neutral-300 hover:text-neutral-900 dark:hover:text-neutral-100 bg-white dark:bg-neutral-900 hover:bg-neutral-100 dark:hover:bg-neutral-800 border border-neutral-200 dark:border-neutral-800 rounded transition-colors shadow-2xs cursor-pointer"
            title="Ask AI to summarize reminders"
          >
            <Sparkles className="w-3.5 h-3.5 text-neutral-500 dark:text-neutral-400" />
            <span>AI Summary</span>
          </button>
          <span className="text-xs font-mono text-neutral-500">
            {data?.items.length || 0} active
          </span>
        </div>
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
              roomLabel={roomLabel}
              timeLeft={calculateTimeLeft(item.trigger_at)}
              onDelete={onDelete}
            />
          ))}
        </div>
      )}
    </div>
  );
};
