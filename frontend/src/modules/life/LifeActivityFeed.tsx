import React from 'react';
import { useQuery } from '@tanstack/react-query';
import { Activity } from 'lucide-react';
import { fetchApi } from '../../core/apiClient';
import type { DayActivityResponse } from './activity-types';

interface LifeActivityFeedProps {
  date: string;
}

const MODULE_LABELS: Record<string, string> = {
  todo: 'Todos',
  finance: 'Finance',
  portfolio: 'Portfolio',
  habit: 'Habits',
  reminder: 'Reminders',
  bookmark: 'Bookmarks',
  footprint: 'Footprints',
};

export const LifeActivityFeed: React.FC<LifeActivityFeedProps> = ({ date }) => {
  const { data, isLoading } = useQuery<DayActivityResponse>({
    queryKey: ['life-day-activity', date],
    queryFn: () => fetchApi<DayActivityResponse>(`/life/days/${date}/activity`),
    staleTime: 30_000,
  });

  if (isLoading) {
    return (
      <div className="border border-neutral-200 dark:border-neutral-800 rounded-lg p-3 bg-white dark:bg-neutral-900">
        <div className="flex items-center gap-1.5 text-xs text-neutral-400 dark:text-neutral-500">
          <Activity className="w-3 h-3 animate-pulse" />
          <span>Loading activity…</span>
        </div>
      </div>
    );
  }

  if (!data || data.events.length === 0) {
    return (
      <div className="border border-neutral-200 dark:border-neutral-800 rounded-lg p-3 bg-white dark:bg-neutral-900">
        <div className="flex items-center gap-1.5">
          <Activity className="w-3 h-3 text-neutral-400 dark:text-neutral-500" />
          <span className="text-xs font-medium text-neutral-600 dark:text-neutral-400">
            Auto-logged Activity
          </span>
        </div>
        <p className="text-[11px] text-neutral-400 dark:text-neutral-500 mt-1 italic">
          No recorded activity from other modules on this day.
        </p>
      </div>
    );
  }

  const { events, stats } = data;

  // Build stat pills for modules that have events
  const statPills: { label: string; count: number }[] = [];
  if (stats.todos_created > 0) statPills.push({ label: 'todos created', count: stats.todos_created });
  if (stats.todos_completed > 0) statPills.push({ label: 'completed', count: stats.todos_completed });
  if (stats.todos_due > 0) statPills.push({ label: 'due', count: stats.todos_due });
  if (stats.expenses > 0) statPills.push({ label: 'expenses', count: stats.expenses });
  if (stats.income > 0) statPills.push({ label: 'income', count: stats.income });
  if (stats.portfolio_trades > 0) statPills.push({ label: 'trades', count: stats.portfolio_trades });
  if (stats.habits_done > 0) statPills.push({ label: 'habits', count: stats.habits_done });
  if (stats.reminders_fired > 0) statPills.push({ label: 'reminders', count: stats.reminders_fired });
  if (stats.bookmarks_saved > 0) statPills.push({ label: 'bookmarks', count: stats.bookmarks_saved });
  if (stats.footprints > 0) statPills.push({ label: 'places', count: stats.footprints });

  return (
    <div className="border border-neutral-200 dark:border-neutral-800 rounded-lg bg-white dark:bg-neutral-900 overflow-hidden">
      {/* Header */}
      <div className="px-3 py-2 border-b border-neutral-100 dark:border-neutral-800 flex items-center justify-between">
        <div className="flex items-center gap-1.5">
          <Activity className="w-3 h-3 text-neutral-500 dark:text-neutral-400" />
          <span className="text-xs font-medium text-neutral-900 dark:text-neutral-100">
            Auto-logged Activity
          </span>
          <span className="text-[10px] font-mono text-neutral-500 dark:text-neutral-400 tabular-nums">
            ({stats.total_events})
          </span>
        </div>
      </div>

      {/* Stat pills */}
      {statPills.length > 0 && (
        <div className="px-3 py-1.5 border-b border-neutral-100 dark:border-neutral-800 flex flex-wrap gap-1">
          {statPills.map((pill) => (
            <span
              key={pill.label}
              className="inline-flex items-center gap-0.5 px-1.5 py-0.5 rounded text-[10px] font-mono bg-neutral-50 dark:bg-neutral-800/60 text-neutral-600 dark:text-neutral-400 border border-neutral-200 dark:border-neutral-700"
            >
              <span className="font-bold tabular-nums">{pill.count}</span>
              <span>{pill.label}</span>
            </span>
          ))}
        </div>
      )}

      {/* Event list */}
      <div className="max-h-52 overflow-y-auto">
        {events.map((event, idx) => (
          <div
            key={idx}
            className={`flex items-start gap-2 px-3 py-1.5 text-xs ${
              idx < events.length - 1
                ? 'border-b border-neutral-50 dark:border-neutral-800/60'
                : ''
            }`}
          >
            {/* Time */}
            <span className="w-10 shrink-0 text-[10px] font-mono text-neutral-400 dark:text-neutral-500 tabular-nums pt-px">
              {event.time ? event.time.slice(0, 5) : '—'}
            </span>

            {/* Emoji */}
            <span className="text-xs shrink-0 pt-px">{event.emoji}</span>

            {/* Content */}
            <div className="flex-1 min-w-0">
              <div className="flex items-baseline gap-1">
                <span className="text-neutral-900 dark:text-neutral-100 font-medium truncate">
                  {event.summary}
                </span>
                <span className="text-[10px] font-mono text-neutral-400 dark:text-neutral-500 shrink-0">
                  {MODULE_LABELS[event.module] || event.module}
                </span>
              </div>
              {event.detail && (
                <p className="text-[10px] text-neutral-500 dark:text-neutral-400 truncate mt-0.5">
                  {event.detail}
                </p>
              )}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
};
