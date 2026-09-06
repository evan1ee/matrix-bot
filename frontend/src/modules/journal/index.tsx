import React, { useState, useEffect, useMemo } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  NotebookPen,
  ChevronLeft,
  ChevronRight,
  Sparkles,
  Clock,
  Trash2,
  Calendar,
} from 'lucide-react';
import { fetchApi } from '../../core/apiClient';
import type { FrontendModule, SystemEvent } from '../../core/types';
import type {
  LifeDayItem,
  MoodType,
  WeatherType,
  UpsertDayPayload,
  TimeAllocation,
} from './types';
import {
  MOOD_CONFIG,
  WEATHER_CONFIG,
  ACTIVITY_OPTIONS,
} from './types';
import { LifeTimeGrid } from '../life/LifeTimeGrid';
import { LifeActivityFeed } from '../life/LifeActivityFeed';
import { DatePicker } from '../../components/DatePicker';

export const JournalView: React.FC = () => {
  const queryClient = useQueryClient();
  const todayStr = useMemo(() => new Date().toISOString().split('T')[0], []);
  const [selectedDate, setSelectedDate] = useState<string>(todayStr);
  const [activeTab, setActiveTab] = useState<'summary' | 'time'>('summary');

  // Form state
  const [selectedMood, setSelectedMood] = useState<MoodType>('good');
  const [selectedWeather, setSelectedWeather] = useState<WeatherType | null>(null);
  const [selectedActivities, setSelectedActivities] = useState<string[]>([]);
  const [note, setNote] = useState('');
  const [timeAllocation, setTimeAllocation] = useState<TimeAllocation>(() => Array(96).fill(null));
  const [timeSaveSuccess, setTimeSaveSuccess] = useState(false);
  const [confirmingClear, setConfirmingClear] = useState(false);
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
  const { data: dayLog, isLoading: isLoadingDay } = useQuery<LifeDayItem | null>({
    queryKey: ['journal-day', selectedDate],
    queryFn: async () => {
      try {
        const res = await fetchApi<any>(`/life/days/${selectedDate}`);
        if (!res) return null;

        let parsedActs: string[] | null = null;
        if (typeof res.activities === 'string') {
          try {
            parsedActs = JSON.parse(res.activities);
          } catch {
            parsedActs = null;
          }
        } else if (Array.isArray(res.activities)) {
          parsedActs = res.activities;
        }

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
          day_type: selectedDate === todayStr ? 'present' : selectedDate < todayStr ? 'past' : 'future',
          mood: res.mood || null,
          weather: res.weather || null,
          activities: parsedActs,
          note: res.note || null,
          time_allocation: parsedTimeAlloc,
        };
      } catch (err: any) {
        if (err?.message?.includes('404')) return null;
        throw err;
      }
    },
  });

  const isLogged = Boolean(dayLog?.mood);
  const loggedMoodCfg = dayLog?.mood ? MOOD_CONFIG[dayLog.mood.toLowerCase() as MoodType] : undefined;
  const loggedWeatherCfg = dayLog?.weather ? WEATHER_CONFIG[dayLog.weather.toLowerCase() as WeatherType] : undefined;

  // Sync form inputs when dayLog data changes
  useEffect(() => {
    if (dayLog) {
      const initialMood = (dayLog.mood?.toLowerCase() as MoodType) || 'good';
      setSelectedMood(initialMood in MOOD_CONFIG ? initialMood : 'good');
      const initialWeather = (dayLog.weather?.toLowerCase() as WeatherType) || null;
      setSelectedWeather(initialWeather && initialWeather in WEATHER_CONFIG ? initialWeather : null);

      let acts: string[] = [];
      if (Array.isArray(dayLog.activities)) {
        acts = dayLog.activities;
      } else if (typeof dayLog.activities === 'string') {
        try {
          acts = JSON.parse(dayLog.activities);
        } catch {
          acts = [];
        }
      }
      setSelectedActivities(acts);
      setNote(dayLog.note || '');

      let parsedTimeAlloc: (string | null)[] = [];
      if (Array.isArray(dayLog.time_allocation)) {
        parsedTimeAlloc = dayLog.time_allocation;
      } else if (typeof dayLog.time_allocation === 'string') {
        try {
          parsedTimeAlloc = JSON.parse(dayLog.time_allocation);
        } catch {
          parsedTimeAlloc = [];
        }
      }

      if (Array.isArray(parsedTimeAlloc) && parsedTimeAlloc.length > 0) {
        const full = [...parsedTimeAlloc];
        while (full.length < 96) full.push(null);
        setTimeAllocation(full);
      } else {
        setTimeAllocation(Array(96).fill(null));
      }
    } else {
      setSelectedMood('good');
      setSelectedWeather(null);
      setSelectedActivities([]);
      setNote('');
      setTimeAllocation(Array(96).fill(null));
    }
    setSaveError(null);
  }, [dayLog, selectedDate]);

  // Reset transient UI state on date change
  useEffect(() => {
    setTimeSaveSuccess(false);
    setConfirmingClear(false);
  }, [selectedDate]);

  // Mutations
  const saveDayMutation = useMutation({
    mutationFn: async (payload: UpsertDayPayload) => {
      return fetchApi('/life/days', {
        method: 'POST',
        body: JSON.stringify({
          log_date: payload.date,
          mood: payload.mood,
          weather: payload.weather ?? null,
          activities: payload.activities ?? null,
          note: payload.note ?? null,
          time_allocation: payload.time_allocation ?? null,
        }),
      });
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['journal-day', selectedDate] });
      queryClient.invalidateQueries({ queryKey: ['life-profile'] });
      queryClient.invalidateQueries({ queryKey: ['life-horizon'] });
    },
  });

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
      queryClient.invalidateQueries({ queryKey: ['journal-day', selectedDate] });
      queryClient.invalidateQueries({ queryKey: ['life-profile'] });
      queryClient.invalidateQueries({ queryKey: ['life-horizon'] });
    },
  });

  const deleteDayMutation = useMutation({
    mutationFn: async (date: string) => {
      return fetchApi(`/life/days/${date}`, { method: 'DELETE' });
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['journal-day', selectedDate] });
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
    return (count * 0.25).toFixed(1);
  }, [timeAllocation]);

  const handleToggleActivity = (actId: string) => {
    setSelectedActivities((prev) =>
      prev.includes(actId) ? prev.filter((a) => a !== actId) : [...prev, actId]
    );
  };

  const handleSaveTimeOnly = async () => {
    setSaveError(null);
    try {
      await saveTimeMutation.mutateAsync({ date: selectedDate, allocation: timeAllocation });
      setTimeSaveSuccess(true);
      setTimeout(() => setTimeSaveSuccess(false), 2500);
    } catch (err: any) {
      setSaveError(err?.message || 'Failed to save time allocation');
    }
  };

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    setSaveError(null);
    try {
      await saveDayMutation.mutateAsync({
        date: selectedDate,
        mood: selectedMood,
        weather: selectedWeather,
        activities: selectedActivities,
        note: note.trim() || null,
        time_allocation: timeAllocation,
      });
    } catch (err: any) {
      setSaveError(err?.message || 'Failed to save daily reflection');
    }
  };

  const handleConfirmDelete = async () => {
    setSaveError(null);
    try {
      await deleteDayMutation.mutateAsync(selectedDate);
      setConfirmingClear(false);
    } catch (err: any) {
      setSaveError(err?.message || 'Failed to clear day record');
    }
  };

  return (
    <div className="space-y-6 pb-12">
      {/* Header & Date Navigation */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 border-b border-zinc-200 dark:border-zinc-800 pb-4">
        <div>
          <div className="flex items-center gap-2.5">
            <h1 className="text-xl font-bold tracking-tight text-zinc-900 dark:text-zinc-100 flex items-center gap-2">
              <NotebookPen className="w-5 h-5 text-zinc-700 dark:text-zinc-300" />
              <span>Daily Log</span>
            </h1>
            {selectedDate === todayStr && (
              <span className="text-[10px] font-mono font-medium px-2 py-0.5 rounded bg-blue-50 dark:bg-blue-950/60 text-blue-700 dark:text-blue-400 border border-blue-200 dark:border-blue-900/60">
                Today
              </span>
            )}
            {isLogged && (
              <span className="text-[10px] font-mono font-medium px-2 py-0.5 rounded bg-emerald-50 dark:bg-emerald-950/60 text-emerald-700 dark:text-emerald-400 border border-emerald-200 dark:border-emerald-900/60">
                Logged
              </span>
            )}
          </div>
          <p className="text-xs text-zinc-500 dark:text-zinc-400 mt-0.5">
            {formattedDate}
          </p>
        </div>

        {/* Date Navigator Controls */}
        <div className="flex items-center gap-1.5">
          <button
            type="button"
            onClick={() => shiftDate(-1)}
            className="h-8 w-8 flex items-center justify-center rounded-md border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900 hover:bg-zinc-50 dark:hover:bg-zinc-800 text-zinc-700 dark:text-zinc-300 transition-colors cursor-pointer shrink-0"
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
            className="h-8 w-8 flex items-center justify-center rounded-md border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900 hover:bg-zinc-50 dark:hover:bg-zinc-800 text-zinc-700 dark:text-zinc-300 transition-colors cursor-pointer shrink-0"
            title="Next day"
          >
            <ChevronRight className="w-4 h-4" />
          </button>

          <button
            type="button"
            onClick={() => setSelectedDate(todayStr)}
            disabled={selectedDate === todayStr}
            className="h-8 px-2.5 rounded-md text-xs font-medium border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900 hover:bg-zinc-50 dark:hover:bg-zinc-800 text-zinc-800 dark:text-zinc-200 transition-colors disabled:opacity-40 disabled:cursor-not-allowed cursor-pointer shrink-0"
          >
            Today
          </button>
        </div>
      </div>

      {/* Tabs */}
      <div className="flex items-center gap-6 border-b border-zinc-200 dark:border-zinc-800 text-xs">
        <button
          type="button"
          onClick={() => setActiveTab('summary')}
          className={`py-2.5 -mb-px flex items-center gap-1.5 border-b-2 font-medium transition-colors cursor-pointer ${
            activeTab === 'summary'
              ? 'border-zinc-900 dark:border-zinc-100 text-zinc-900 dark:text-zinc-100'
              : 'border-transparent text-zinc-500 dark:text-zinc-400 hover:text-zinc-700 dark:hover:text-zinc-300'
          }`}
        >
          <Sparkles className="w-3.5 h-3.5" />
          <span>Day Summary & Activities</span>
        </button>

        <button
          type="button"
          onClick={() => setActiveTab('time')}
          className={`py-2.5 -mb-px flex items-center gap-1.5 border-b-2 font-medium transition-colors cursor-pointer ${
            activeTab === 'time'
              ? 'border-zinc-900 dark:border-zinc-100 text-zinc-900 dark:text-zinc-100'
              : 'border-transparent text-zinc-500 dark:text-zinc-400 hover:text-zinc-700 dark:hover:text-zinc-300'
          }`}
        >
          <Clock className="w-3.5 h-3.5" />
          <span>24h Time Grid</span>
          {Number(timeTrackedHours) > 0 && (
            <span className="ml-1 text-[10px] font-mono px-1.5 py-0.5 rounded bg-zinc-100 dark:bg-zinc-800 text-zinc-600 dark:text-zinc-400">
              {timeTrackedHours}h
            </span>
          )}
        </button>
      </div>

      {/* Error Banner */}
      {saveError && (
        <div className="p-3 rounded-lg bg-rose-50 dark:bg-rose-950/40 border border-rose-200 dark:border-rose-900 text-xs text-rose-700 dark:text-rose-300 flex items-start gap-2">
          <span className="shrink-0 font-bold">⚠️</span>
          <span className="flex-1">{saveError}</span>
        </div>
      )}

      {/* Tab 1: Day Summary & Auto-Logged Activity Feed */}
      {activeTab === 'summary' && (
        <div className="grid grid-cols-1 lg:grid-cols-12 gap-8 items-start">
          {/* Left Column: Reflection Form or Sealed View */}
          <div className="lg:col-span-6 space-y-6">
            {isLogged ? (
              /* Sealed/Logged View */
              <div className="border border-zinc-200 dark:border-zinc-800 rounded-lg p-5 bg-white dark:bg-zinc-900 space-y-4">
                <div className="flex items-center justify-between">
                  <span className="text-xs font-semibold text-zinc-900 dark:text-zinc-100">
                    Sealed Daily Reflection
                  </span>
                  <span className="text-[10px] font-mono text-emerald-600 dark:text-emerald-400">
                    Recorded
                  </span>
                </div>

                {/* Mood Card */}
                <div
                  className={`flex items-center gap-3 p-3.5 rounded-lg border ${
                    loggedMoodCfg
                      ? `${loggedMoodCfg.bgClass} ${loggedMoodCfg.textClass} font-semibold shadow-xs`
                      : 'bg-zinc-50 dark:bg-zinc-800/50 border-zinc-200 dark:border-zinc-800 text-zinc-800 dark:text-zinc-200'
                  }`}
                >
                  <span className="text-3xl">{loggedMoodCfg?.emoji || '✨'}</span>
                  <div>
                    <div className="text-base font-bold capitalize">{loggedMoodCfg?.label || dayLog?.mood}</div>
                    <div className="text-xs opacity-80">Recorded mood for this day</div>
                  </div>
                </div>

                {/* Weather */}
                {dayLog?.weather && (
                  <div>
                    <div className="text-[10px] font-mono font-medium uppercase tracking-wider text-zinc-500 mb-1">
                      Weather
                    </div>
                    <div className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-zinc-50 dark:bg-zinc-800/60 border border-zinc-200 dark:border-zinc-700 text-xs font-medium text-zinc-800 dark:text-zinc-200">
                      <span className="text-sm">{loggedWeatherCfg?.emoji || '🌤️'}</span>
                      <span>{loggedWeatherCfg?.label || dayLog.weather}</span>
                    </div>
                  </div>
                )}

                {/* Activities */}
                {dayLog?.activities && dayLog.activities.length > 0 && (
                  <div>
                    <div className="text-[10px] font-mono font-medium uppercase tracking-wider text-zinc-500 mb-1">
                      Activities & Milestones
                    </div>
                    <div className="flex flex-wrap gap-1.5">
                      {dayLog.activities.map((actId) => {
                        const act = ACTIVITY_OPTIONS.find((a) => a.id === actId);
                        return (
                          <span
                            key={actId}
                            className="inline-flex items-center gap-1 px-2.5 py-1 rounded-md text-xs font-medium bg-zinc-50 dark:bg-zinc-800/60 border border-zinc-200 dark:border-zinc-700 text-zinc-800 dark:text-zinc-200"
                          >
                            <span>{act?.emoji || '📌'}</span>
                            <span>{act?.label || actId}</span>
                          </span>
                        );
                      })}
                    </div>
                  </div>
                )}

                {/* Reflection Note */}
                <div>
                  <div className="text-[10px] font-mono font-medium uppercase tracking-wider text-zinc-500 mb-1">
                    Notes & Reflection
                  </div>
                  {dayLog?.note ? (
                    <div className="p-3.5 rounded-lg bg-zinc-50 dark:bg-zinc-800/50 border border-zinc-200 dark:border-zinc-800 text-xs text-zinc-800 dark:text-zinc-200 whitespace-pre-wrap leading-relaxed">
                      "{dayLog.note}"
                    </div>
                  ) : (
                    <div className="p-3 rounded-md bg-zinc-50/50 dark:bg-zinc-800/30 border border-zinc-200/80 dark:border-zinc-800/80 text-xs text-zinc-400 dark:text-zinc-500 italic">
                      No reflection notes were recorded for this day.
                    </div>
                  )}
                </div>

                {/* Clear Confirmation */}
                {confirmingClear ? (
                  <div className="p-3 rounded-lg border border-rose-200 dark:border-rose-900/60 bg-rose-50/50 dark:bg-rose-950/20 space-y-2.5">
                    <p className="text-xs text-rose-800 dark:text-rose-200 font-medium">
                      Clear reflection record for {selectedDate}?
                    </p>
                    <div className="flex items-center justify-end gap-2">
                      <button
                        type="button"
                        onClick={() => setConfirmingClear(false)}
                        className="px-2.5 py-1 text-xs text-zinc-600 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-800 rounded"
                      >
                        Cancel
                      </button>
                      <button
                        type="button"
                        onClick={handleConfirmDelete}
                        disabled={deleteDayMutation.isPending}
                        className="px-3 py-1 text-xs font-medium bg-rose-600 hover:bg-rose-700 text-white rounded cursor-pointer"
                      >
                        {deleteDayMutation.isPending ? 'Clearing...' : 'Yes, clear record'}
                      </button>
                    </div>
                  </div>
                ) : (
                  <div className="pt-2 border-t border-zinc-100 dark:border-zinc-800 flex justify-end">
                    <button
                      type="button"
                      onClick={() => setConfirmingClear(true)}
                      className="inline-flex items-center gap-1.5 text-xs text-rose-600 dark:text-rose-400 hover:underline cursor-pointer"
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                      <span>Clear record</span>
                    </button>
                  </div>
                )}
              </div>
            ) : (
              /* Editable Reflection Form */
              <form
                onSubmit={handleSave}
                className="border border-zinc-200 dark:border-zinc-800 rounded-lg p-5 bg-white dark:bg-zinc-900 space-y-4"
              >
                <div className="flex items-center justify-between">
                  <span className="text-xs font-semibold text-zinc-900 dark:text-zinc-100">
                    Record Daily Reflection
                  </span>
                  <span className="text-[10px] font-mono text-zinc-400">
                    Unrecorded
                  </span>
                </div>

                {/* Mood Selector */}
                <div>
                  <label className="block text-xs font-medium text-zinc-600 dark:text-zinc-400 mb-1.5">
                    Primary Mood
                  </label>
                  <div className="grid grid-cols-5 gap-2">
                    {(Object.keys(MOOD_CONFIG) as MoodType[]).map((moodKey) => {
                      const config = MOOD_CONFIG[moodKey];
                      const isSelected = selectedMood === moodKey;
                      return (
                        <button
                          key={moodKey}
                          type="button"
                          onClick={() => setSelectedMood(moodKey)}
                          className={`flex flex-col items-center justify-center p-2 rounded-lg border text-xs transition-all cursor-pointer ${
                            isSelected
                              ? `${config.bgClass} ${config.textClass} font-semibold ring-1 ring-zinc-900 dark:ring-zinc-100 shadow-xs`
                              : 'border-zinc-200 dark:border-zinc-800 bg-zinc-50 dark:bg-zinc-800/50 text-zinc-700 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-800'
                          }`}
                        >
                          <span className="text-2xl mb-1">{config.emoji}</span>
                          <span className="text-[11px]">{config.label}</span>
                        </button>
                      );
                    })}
                  </div>
                </div>

                {/* Weather */}
                <div>
                  <label className="block text-xs font-medium text-zinc-600 dark:text-zinc-400 mb-1">
                    Weather (optional)
                  </label>
                  <div className="flex flex-wrap gap-1.5">
                    {(Object.keys(WEATHER_CONFIG) as WeatherType[]).map((wKey) => {
                      const w = WEATHER_CONFIG[wKey];
                      const isSelected = selectedWeather === wKey;
                      return (
                        <button
                          key={wKey}
                          type="button"
                          onClick={() => setSelectedWeather(isSelected ? null : wKey)}
                          className={`inline-flex items-center gap-1 px-2.5 py-1 rounded-md text-xs font-medium border transition-colors cursor-pointer ${
                            isSelected
                              ? 'bg-zinc-900 text-white dark:bg-zinc-100 dark:text-zinc-900 border-zinc-900 dark:border-zinc-100'
                              : 'border-zinc-200 dark:border-zinc-800 bg-zinc-50 dark:bg-zinc-800/40 text-zinc-600 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-800'
                          }`}
                        >
                          <span>{w.emoji}</span>
                          <span>{w.label}</span>
                        </button>
                      );
                    })}
                  </div>
                </div>

                {/* Activities */}
                <div>
                  <label className="block text-xs font-medium text-zinc-600 dark:text-zinc-400 mb-1">
                    Activities & milestones
                  </label>
                  <div className="grid grid-cols-2 gap-1.5">
                    {ACTIVITY_OPTIONS.map((act) => {
                      const isChecked = selectedActivities.includes(act.id);
                      return (
                        <button
                          key={act.id}
                          type="button"
                          onClick={() => handleToggleActivity(act.id)}
                          className={`flex items-center gap-2 px-2.5 py-1.5 rounded-md text-xs font-medium border transition-colors text-left cursor-pointer ${
                            isChecked
                              ? 'bg-zinc-900 text-white dark:bg-zinc-100 dark:text-zinc-900 border-zinc-900 dark:border-zinc-100'
                              : 'border-zinc-200 dark:border-zinc-800 bg-zinc-50 dark:bg-zinc-800/40 text-zinc-600 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-800'
                          }`}
                        >
                          <span>{act.emoji}</span>
                          <span className="truncate">{act.label}</span>
                        </button>
                      );
                    })}
                  </div>
                </div>

                {/* Notes */}
                <div>
                  <label className="block text-xs font-medium text-zinc-600 dark:text-zinc-400 mb-1">
                    Notes & reflection (optional)
                  </label>
                  <textarea
                    value={note}
                    onChange={(e) => setNote(e.target.value)}
                    placeholder="What happened today? Memories, achievements, or learnings..."
                    maxLength={500}
                    rows={3}
                    className="w-full px-3 py-2 rounded-md text-xs border border-zinc-200 dark:border-zinc-700 bg-zinc-50/50 dark:bg-zinc-800/50 text-zinc-900 dark:text-zinc-100 placeholder-zinc-400 focus:outline-hidden focus:ring-1 focus:ring-zinc-900 dark:focus:ring-zinc-100 resize-none"
                  />
                  <div className="text-right text-[10px] text-zinc-400 mt-0.5 tabular-nums">
                    {note.length}/500
                  </div>
                </div>

                {/* Submit Action */}
                <div className="pt-2 border-t border-zinc-100 dark:border-zinc-800 flex justify-end">
                  <button
                    type="submit"
                    disabled={saveDayMutation.isPending}
                    className="h-9 px-4 text-xs font-medium text-white bg-zinc-900 hover:bg-zinc-800 dark:bg-zinc-100 dark:text-zinc-900 dark:hover:bg-white rounded-md shadow-xs transition-colors disabled:opacity-50 cursor-pointer inline-flex items-center gap-1.5"
                  >
                    {saveDayMutation.isPending && (
                      <svg className="animate-spin -ml-0.5 mr-1 h-3 w-3 text-white dark:text-zinc-900" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24">
                        <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4"></circle>
                        <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
                      </svg>
                    )}
                    <span>{saveDayMutation.isPending ? 'Saving...' : 'Save daily reflection'}</span>
                  </button>
                </div>
              </form>
            )}
          </div>

          {/* Right Column: Auto-Logged Activity Feed & Time Overview */}
          <div className="lg:col-span-6 space-y-6">
            <LifeActivityFeed date={selectedDate} />

            {/* Time Grid Quick Jump Card */}
            <div className="p-4 rounded-lg border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900 flex items-center justify-between">
              <div>
                <div className="text-xs font-medium text-zinc-900 dark:text-zinc-100">
                  {Number(timeTrackedHours) > 0
                    ? `${timeTrackedHours}h Tracked in 24-Hour Timeline`
                    : '24-Hour Time Allocation'}
                </div>
                <div className="text-[11px] text-zinc-500 mt-0.5">
                  {Number(timeTrackedHours) > 0
                    ? 'Mapped across 15-minute intervals'
                    : 'Map deep work, sleep & leisure blocks'}
                </div>
              </div>
              <button
                type="button"
                onClick={() => setActiveTab('time')}
                className="px-3 py-1.5 rounded-md text-xs font-medium border border-zinc-200 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-800 hover:bg-zinc-100 dark:hover:bg-zinc-700 text-zinc-800 dark:text-zinc-200 transition-colors cursor-pointer"
              >
                {Number(timeTrackedHours) > 0 ? 'Edit Blocks →' : 'Open Grid →'}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Tab 2: 24-Hour Time Grid */}
      {activeTab === 'time' && (
        <div className="border border-zinc-200 dark:border-zinc-800 rounded-lg p-5 bg-white dark:bg-zinc-900 space-y-5">
          <LifeTimeGrid
            timeAllocation={timeAllocation}
            onChange={setTimeAllocation}
            readOnly={selectedDate !== todayStr && isLogged}
            isToday={selectedDate === todayStr}
          />
          <div className="flex items-center justify-between pt-3 border-t border-zinc-100 dark:border-zinc-800">
            <span className="text-xs text-zinc-500">
              {timeSaveSuccess
                ? '✓ Time blocks saved'
                : selectedDate === todayStr
                ? 'Update your intraday blocks anytime today'
                : isLogged
                ? 'Past day time blocks are sealed'
                : 'Save time blocks or switch to Day Summary'}
            </span>
            <div className="flex items-center gap-2.5">
              {(!isLogged || selectedDate === todayStr) && (
                <button
                  type="button"
                  disabled={saveTimeMutation.isPending}
                  onClick={handleSaveTimeOnly}
                  className="px-3.5 py-1.5 rounded-md text-xs font-medium border border-zinc-200 dark:border-zinc-700 bg-zinc-900 text-white dark:bg-zinc-100 dark:text-zinc-900 hover:bg-zinc-800 dark:hover:bg-white transition-colors cursor-pointer disabled:opacity-50"
                >
                  {saveTimeMutation.isPending ? 'Saving...' : timeSaveSuccess ? '✓ Saved' : 'Save time blocks'}
                </button>
              )}
              <button
                type="button"
                onClick={() => setActiveTab('summary')}
                className="px-3.5 py-1.5 rounded-md text-xs font-medium border border-zinc-200 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-800 hover:bg-zinc-100 dark:hover:bg-zinc-700 text-zinc-800 dark:text-zinc-200 transition-colors cursor-pointer"
              >
                ← Back to Day Summary
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};

export const journalModule: FrontendModule = {
  id: 'journal',
  name: 'Daily Log',
  description: 'Daily Reflection, 24-Hour Time Allocation, and Auto-logged Activities',
  iconName: 'NotebookPen',
  navTab: {
    label: 'Daily Log',
  },
  MainView: JournalView,
  handleSseEvent: (event: SystemEvent, queryClient) => {
    if (event.type === 'LifeDayLogged' || event.type === 'LifeDayCleared') {
      queryClient.invalidateQueries({ queryKey: ['journal-day'] });
    }
  },
};
