import React, { useState, useEffect, useMemo, useRef } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import { todayInZone, useSettings } from '../../core/settings';
import type { FrontendModule, SystemEvent } from '../../core/types';
import type {
  LifeDayItem,
  MoodType,
  WeatherType,
  UpsertDayPayload,
} from '../life/types';
import { MoodHeader } from './components/MoodHeader';
import { FeelAndEnvironmentCard } from './components/FeelAndEnvironmentCard';
import { ReflectionCard } from './components/ReflectionCard';

export const MoodView: React.FC = () => {
  const queryClient = useQueryClient();
  const { timezone } = useSettings();
  const todayStr = useMemo(() => todayInZone(timezone), [timezone]);
  const [selectedDate, setSelectedDate] = useState<string>(() => todayInZone(timezone));
  const prevTodayRef = useRef(todayStr);

  useEffect(() => {
    if (selectedDate === prevTodayRef.current && todayStr !== prevTodayRef.current) {
      setSelectedDate(todayStr);
    }
    prevTodayRef.current = todayStr;
  }, [todayStr, selectedDate]);

  // Form state
  const [selectedMood, setSelectedMood] = useState<MoodType>('good');
  const [selectedWeather, setSelectedWeather] = useState<WeatherType | null>(null);
  const [selectedActivities, setSelectedActivities] = useState<string[]>([]);
  const [note, setNote] = useState('');
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
  const { data: dayLog } = useQuery<LifeDayItem | null>({
    queryKey: ['mood-day', selectedDate],
    queryFn: async () => {
      try {
        const res = await fetchApi<any>(`/life/days/${selectedDate}`);
        if (!res) return null;

        let acts: string[] = [];
        if (Array.isArray(res.activities)) {
          acts = res.activities;
        } else if (typeof res.activities === 'string') {
          try {
            acts = JSON.parse(res.activities);
          } catch {
            acts = [];
          }
        }

        const dayType =
          selectedDate === todayStr
            ? 'present'
            : selectedDate < todayStr
              ? 'past'
              : 'future';

        return {
          date: res.log_date,
          day_type: dayType,
          mood: res.mood,
          weather: res.weather,
          activities: acts,
          note: res.note,
          time_allocation: res.time_allocation,
        };
      } catch (err: any) {
        if (err?.status === 404 || err?.message?.includes('not found')) {
          return null;
        }
        throw err;
      }
    },
  });

  const isLogged = Boolean(dayLog?.mood);
  const isFormActive = !isLogged;

  // Sync loaded dayLog into form state
  useEffect(() => {
    if (dayLog && dayLog.mood) {
      setSelectedMood(dayLog.mood.toLowerCase() as MoodType);
      setSelectedWeather((dayLog.weather?.toLowerCase() as WeatherType) || null);
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
    } else {
      setSelectedMood('good');
      setSelectedWeather(null);
      setSelectedActivities([]);
      setNote('');
    }
    setSaveError(null);
    setConfirmingClear(false);
  }, [dayLog, selectedDate]);

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
        }),
      });
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['mood-day', selectedDate] });
      queryClient.invalidateQueries({ queryKey: ['life-profile'] });
      queryClient.invalidateQueries({ queryKey: ['life-horizon'] });
    },
  });

  const deleteDayMutation = useMutation({
    mutationFn: async (date: string) => {
      return fetchApi(`/life/days/${date}`, { method: 'DELETE' });
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['mood-day', selectedDate] });
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

  const handleToggleActivity = (id: string) => {
    setSelectedActivities((prev) =>
      prev.includes(id) ? prev.filter((a) => a !== id) : [...prev, id]
    );
  };

  const handleSave = async (e?: React.FormEvent) => {
    if (e) e.preventDefault();
    setSaveError(null);
    try {
      await saveDayMutation.mutateAsync({
        date: selectedDate,
        mood: selectedMood,
        weather: selectedWeather,
        activities: selectedActivities,
        note: note.trim() || undefined,
      });
    } catch (err: any) {
      setSaveError(err?.message || 'Failed to save reflection');
    }
  };

  const handleConfirmDelete = async () => {
    try {
      await deleteDayMutation.mutateAsync(selectedDate);
      setConfirmingClear(false);
    } catch (err: any) {
      setSaveError(err?.message || 'Failed to clear day log');
    }
  };

  const formattedDate = useMemo(() => {
    const d = new Date(`${selectedDate}T12:00:00Z`);
    return d.toLocaleDateString('en-US', {
      weekday: 'long',
      month: 'long',
      day: 'numeric',
      year: 'numeric',
    });
  }, [selectedDate]);

  return (
    <div className="space-y-6">
      {/* Header & Date Navigation */}
      <MoodHeader
        formattedDate={formattedDate}
        selectedDate={selectedDate}
        todayStr={todayStr}
        isLogged={isLogged}
        onShiftDate={shiftDate}
        onSelectDate={setSelectedDate}
        onSelectToday={() => setSelectedDate(todayStr)}
      />

      {/* Error Banner */}
      {saveError && (
        <div className="p-3 rounded-lg bg-rose-50 dark:bg-rose-950/40 border border-rose-200 dark:border-rose-900 text-xs text-rose-700 dark:text-rose-300 flex items-start gap-2">
          <span className="shrink-0 font-bold">⚠️</span>
          <span className="flex-1">{saveError}</span>
        </div>
      )}

      {/* Redesigned Studio Layout: Left Vibe & Milestones, Right Reflection Prose & Actions */}
      <div className="grid grid-cols-1 lg:grid-cols-12 gap-6 items-start">
        {/* Left Column: Mood, Weather, Activities & Milestones */}
        <div className="lg:col-span-6 space-y-6">
          <FeelAndEnvironmentCard
            isFormActive={isFormActive}
            dayLog={dayLog}
            selectedMood={selectedMood}
            onSelectMood={setSelectedMood}
            selectedWeather={selectedWeather}
            onSelectWeather={setSelectedWeather}
            selectedActivities={selectedActivities}
            onToggleActivity={handleToggleActivity}
          />
        </div>

        {/* Right Column: Reflection Notes & Actions */}
        <div className="lg:col-span-6 space-y-6">
          <ReflectionCard
            isFormActive={isFormActive}
            dayLog={dayLog}
            selectedDate={selectedDate}
            note={note}
            onChangeNote={setNote}
            onSubmit={handleSave}
            isSaving={saveDayMutation.isPending}
            confirmingClear={confirmingClear}
            onSetConfirmingClear={setConfirmingClear}
            onConfirmDelete={handleConfirmDelete}
            isDeleting={deleteDayMutation.isPending}
          />
        </div>
      </div>
    </div>
  );
};

export const moodModule: FrontendModule = {
  id: 'mood',
  name: 'Mood',
  description: 'Track daily feel, weather, notes, and activity highlights',
  iconName: 'Smile',
  navTab: {
    label: 'Mood',
  },
  MainView: MoodView,
  handleSseEvent: (event: SystemEvent, queryClient) => {
    if (event.type === 'LifeDayLogged' || event.type === 'LifeDayCleared') {
      queryClient.invalidateQueries({ queryKey: ['mood-day'] });
    }
  },
};
