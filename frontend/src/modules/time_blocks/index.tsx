import React, { useState, useEffect, useMemo } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  Clock,
  ChevronLeft,
  ChevronRight,
} from 'lucide-react';
import { fetchApi } from '../../core/apiClient';
import type { FrontendModule, SystemEvent } from '../../core/types';
import type { TimeAllocation } from '../life/types';
import { LifeTimeGrid } from '../life/LifeTimeGrid';
import { DatePicker } from '../../components/DatePicker';

export const TimeBlocksView: React.FC = () => {
  const queryClient = useQueryClient();
  const todayStr = useMemo(() => new Date().toISOString().split('T')[0], []);
  const [selectedDate, setSelectedDate] = useState<string>(todayStr);

  const [timeAllocation, setTimeAllocation] = useState<TimeAllocation>(() => Array(288).fill(null));
  const [timeSaveSuccess, setTimeSaveSuccess] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  // Listen for navigation events with specific dates
  useEffect(() => {
    const onNavigate = (e: Event) => {
      const detail = (e as CustomEvent).detail;
      if (detail?.date) {
        setSelectedDate(detail.date);
      }
    };
    window.addEventListener('app:navigate', onNavigate);
    return () => window.removeEventListener('app:navigate', onNavigate);
  }, []);

  // Fetch day log for selectedDate
  const { data: dayLog } = useQuery({
    queryKey: ['time-blocks-day', selectedDate],
    queryFn: async () => {
      try {
        const res = await fetchApi<any>(`/life/days/${selectedDate}`);
        if (!res) return null;

        let parsedTimeAlloc: TimeAllocation | null = null;
        if (typeof res.time_allocation === 'string') {
          try {
            parsedTimeAlloc = JSON.parse(res.time_allocation);
          } catch {
            parsedTimeAlloc = null;
          }
        } else if (Array.isArray(res.time_allocation)) {
          parsedTimeAlloc = res.time_allocation;
        }

        return {
          date: res.log_date || selectedDate,
          mood: res.mood || null,
          time_allocation: parsedTimeAlloc,
        };
      } catch (err: any) {
        if (err?.message?.includes('404')) return null;
        throw err;
      }
    },
  });

  const isLogged = Boolean(dayLog?.mood);

  // Sync time allocation state on dayLog change
  useEffect(() => {
    if (dayLog && Array.isArray(dayLog.time_allocation) && dayLog.time_allocation.length > 0) {
      const full = [...dayLog.time_allocation];
      while (full.length < 288) full.push(null);
      setTimeAllocation(full);
    } else {
      setTimeAllocation(Array(288).fill(null));
    }
    setTimeSaveSuccess(false);
    setSaveError(null);
  }, [dayLog, selectedDate]);

  // Mutations
  const saveTimeMutation = useMutation({
    mutationFn: async ({ date, allocation }: { date: string; allocation: TimeAllocation }) => {
      return fetchApi(`/life/days/${date}/time`, {
        method: 'PUT',
        body: JSON.stringify({
          time_allocation: allocation,
        }),
      });
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['time-blocks-day', selectedDate] });
      queryClient.invalidateQueries({ queryKey: ['life-profile'] });
      queryClient.invalidateQueries({ queryKey: ['life-horizon'] });
    },
  });

  // Date Navigation Helpers
  const shiftDate = (days: number) => {
    const cur = new Date(`${selectedDate}T12:00:00Z`);
    cur.setUTCDate(cur.getUTCDate() + days);
    setSelectedDate(cur.toISOString().split('T')[0]);
  };

  const formattedDate = useMemo(() => {
    return new Date(`${selectedDate}T12:00:00Z`).toLocaleDateString('en-US', {
      weekday: 'long',
      month: 'long',
      day: 'numeric',
      year: 'numeric',
    });
  }, [selectedDate]);

  const timeTrackedHours = useMemo(() => {
    const count = timeAllocation.filter(Boolean).length;
    return ((count * 5) / 60).toFixed(1);
  }, [timeAllocation]);

  const handleSaveTime = async () => {
    setSaveError(null);
    try {
      await saveTimeMutation.mutateAsync({ date: selectedDate, allocation: timeAllocation });
      setTimeSaveSuccess(true);
      setTimeout(() => setTimeSaveSuccess(false), 2500);
    } catch (err: any) {
      setSaveError(err?.message || 'Failed to save time allocation');
    }
  };


  return (
    <div className="space-y-6">
      {/* Header & Date Navigation */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 border-b border-neutral-200 dark:border-neutral-800 pb-4">
        <div className="flex items-center gap-2.5">
          <Clock className="w-4 h-4 text-neutral-600 dark:text-neutral-300" />
          <h2 className="text-base font-semibold text-neutral-900 dark:text-neutral-100 font-sans">
            {formattedDate}
          </h2>
          {selectedDate === todayStr && (
            <span className="text-[10px] font-mono font-medium px-2 py-0.5 rounded bg-blue-50 dark:bg-blue-950/60 text-blue-700 dark:text-blue-400 border border-blue-200 dark:border-blue-900/60">
              Today
            </span>
          )}
          {Number(timeTrackedHours) > 0 && (
            <span className="text-[10px] font-mono font-medium px-2 py-0.5 rounded bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400">
              {timeTrackedHours}h / 24h Tracked
            </span>
          )}
        </div>

        {/* Date Navigator Controls */}
        <div className="flex items-center gap-1.5">
          <div className="flex items-center gap-1">
            <button
              type="button"
              onClick={() => shiftDate(-1)}
              className="h-8 w-8 flex items-center justify-center rounded-md border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 hover:bg-neutral-50 dark:hover:bg-neutral-800 hover:border-neutral-300 dark:hover:border-neutral-700 text-neutral-700 dark:text-neutral-300 transition-colors duration-150 cursor-pointer shrink-0 shadow-2xs"
              title="Previous day"
            >
              <ChevronLeft className="w-4 h-4" />
            </button>

            <DatePicker
              value={selectedDate}
              onChange={(val) => val && setSelectedDate(val)}
              size="sm"
              align="right"
              required
              className="w-34 sm:w-36 shrink-0"
            />

            <button
              type="button"
              onClick={() => shiftDate(1)}
              className="h-8 w-8 flex items-center justify-center rounded-md border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 hover:bg-neutral-50 dark:hover:bg-neutral-800 hover:border-neutral-300 dark:hover:border-neutral-700 text-neutral-700 dark:text-neutral-300 transition-colors duration-150 cursor-pointer shrink-0 shadow-2xs"
              title="Next day"
            >
              <ChevronRight className="w-4 h-4" />
            </button>
          </div>

          {selectedDate !== todayStr && (
            <button
              type="button"
              onClick={() => setSelectedDate(todayStr)}
              className="h-8 px-2.5 rounded-md text-xs font-medium border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 hover:bg-neutral-50 dark:hover:bg-neutral-800 hover:border-neutral-300 dark:hover:border-neutral-700 text-neutral-800 dark:text-neutral-200 transition-colors duration-150 cursor-pointer shrink-0 inline-flex items-center gap-1 shadow-2xs"
            >
              <span>Back to today</span>
            </button>
          )}
        </div>
      </div>

      {/* Error Banner */}
      {saveError && (
        <div className="p-3 rounded-lg bg-rose-50 dark:bg-rose-950/40 border border-rose-200 dark:border-rose-900 text-xs text-rose-700 dark:text-rose-300 flex items-start gap-2">
          <span className="shrink-0 font-bold">⚠️</span>
          <span className="flex-1">{saveError}</span>
        </div>
      )}

      {/* Time Allocation Grid Card */}
      <div className="border border-neutral-200 dark:border-neutral-800 rounded-lg p-5 bg-white dark:bg-neutral-900 space-y-5 shadow-2xs">
        <LifeTimeGrid
          timeAllocation={timeAllocation}
          onChange={setTimeAllocation}
          readOnly={selectedDate !== todayStr && isLogged}
          isToday={selectedDate === todayStr}
        />

        <div className="flex items-center justify-between pt-3 border-t border-neutral-100 dark:border-neutral-800">
          <span className="text-xs text-neutral-500 font-mono">
            {timeSaveSuccess
              ? '✓ Time blocks saved'
              : selectedDate === todayStr
              ? 'Update your intraday blocks anytime today'
              : isLogged
              ? 'Past day time blocks are sealed'
              : 'Allocate your 24 hours across deep work, sleep, and leisure'}
          </span>

          <div className="flex items-center gap-2.5">
            {(!isLogged || selectedDate === todayStr) && (
              <button
                type="button"
                disabled={saveTimeMutation.isPending}
                onClick={handleSaveTime}
                className="px-4 py-1.5 rounded-md text-xs font-medium bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900 hover:bg-neutral-800 dark:hover:bg-neutral-200 transition-colors duration-150 cursor-pointer disabled:opacity-50 shadow-2xs"
              >
                {saveTimeMutation.isPending ? 'Saving...' : timeSaveSuccess ? '✓ Saved' : 'Save time blocks'}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
};

export const timeBlocksModule: FrontendModule = {
  id: 'time_blocks',
  name: 'Time Blocks',
  description: '24-Hour visual time allocation grid across 5-minute intervals',
  iconName: 'Clock',
  navTab: {
    label: 'Time Blocks',
  },
  MainView: TimeBlocksView,
  handleSseEvent: (event: SystemEvent, queryClient) => {
    if (event.type === 'LifeDayLogged' || event.type === 'LifeDayCleared') {
      queryClient.invalidateQueries({ queryKey: ['time-blocks-day'] });
    }
  },
};
