import React, { useState, useMemo } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Hourglass, Plus, Calendar } from 'lucide-react';
import type { FrontendModule, SystemEvent } from '../../core/types';
import type {
  LifeProfileResponse,
  LifeProfileStats,
  LifeHorizonDay,
  LifeDayItem,
  UpsertDayPayload,
  UpsertProfilePayload,
} from './types';
import { LifeStatStrip } from './LifeStatStrip';
import { LifeHorizonMinimap } from './LifeHorizonMinimap';
import { LifeAllBlocksGrid } from './LifeAllBlocksGrid';
import { LifeDayModal } from './LifeDayModal';
import { LifeProfileModal } from './LifeProfileModal';

export const LifeView: React.FC = () => {
  const queryClient = useQueryClient();
  const currentActualYear = new Date().getFullYear();
  const [selectedDate, setSelectedDate] = useState<string | null>(null);
  const [selectedDay, setSelectedDay] = useState<LifeDayItem | null>(null);
  const [isDayModalOpen, setIsDayModalOpen] = useState(false);
  const [isProfileModalOpen, setIsProfileModalOpen] = useState(false);

  const isDark =
    typeof document !== 'undefined' &&
    document.documentElement.classList.contains('dark');

  // 1. Fetch Profile & Overall Stats
  const { data: profileData, isLoading: isLoadingProfile } = useQuery<LifeProfileResponse>({
    queryKey: ['life-profile'],
    queryFn: async () => {
      const res = await fetch('/api/v1/life/profile', { credentials: 'include' });
      if (!res.ok) throw new Error('Failed to load life profile');
      return res.json();
    },
  });

  // 2. Fetch Macro Life Horizon Array (All ~30,000 days of life)
  const { data: horizonDays = [] } = useQuery<LifeHorizonDay[]>({
    queryKey: ['life-horizon'],
    enabled: !!profileData?.profile,
    queryFn: async () => {
      const res = await fetch('/api/v1/life/horizon', { credentials: 'include' });
      if (!res.ok) throw new Error('Failed to load life horizon');
      return res.json();
    },
  });

  // Mutations
  const saveProfileMutation = useMutation({
    mutationFn: async (payload: UpsertProfilePayload) => {
      const res = await fetch('/api/v1/life/profile', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'include',
        body: JSON.stringify(payload),
      });
      if (!res.ok) {
        const data = await res.json().catch(() => ({}));
        throw new Error(data?.error || 'Failed to save life profile');
      }
      return res.json();
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['life-profile'] });
      queryClient.invalidateQueries({ queryKey: ['life-horizon'] });
    },
  });

  const saveDayMutation = useMutation({
    mutationFn: async (payload: UpsertDayPayload) => {
      const res = await fetch('/api/v1/life/days', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'include',
        body: JSON.stringify({
          ...payload,
          log_date: payload.log_date || payload.date,
          date: payload.date || payload.log_date,
        }),
      });
      if (!res.ok) {
        const data = await res.json().catch(() => ({}));
        throw new Error(data?.error || 'Failed to save day record');
      }
      return res.json();
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['life-profile'] });
      queryClient.invalidateQueries({ queryKey: ['life-horizon'] });
    },
  });

  const deleteDayMutation = useMutation({
    mutationFn: async (date: string) => {
      const res = await fetch(`/api/v1/life/days/${date}`, {
        method: 'DELETE',
        credentials: 'include',
      });
      if (!res.ok) {
        const data = await res.json().catch(() => ({}));
        throw new Error(data?.error || 'Failed to delete day record');
      }
      return res.json();
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['life-profile'] });
      queryClient.invalidateQueries({ queryKey: ['life-horizon'] });
    },
  });

  const handleSelectDate = async (date: string) => {
    setSelectedDate(date);
    const todayStr = new Date().toISOString().split('T')[0];
    let dayType: 'past' | 'present' | 'future' = 'past';
    if (date === todayStr) {
      dayType = 'present';
    } else if (date > todayStr) {
      dayType = 'future';
    }

    try {
      const res = await fetch(`/api/v1/life/days/${date}`, { credentials: 'include' });
      if (res.ok) {
        const log = await res.json();
        if (log) {
          let acts: string[] = [];
          try {
            if (log.activities) acts = JSON.parse(log.activities);
          } catch {
            acts = [];
          }
          setSelectedDay({
            date: log.log_date,
            day_type: dayType,
            mood: log.mood,
            weather: log.weather,
            activities: acts,
            note: log.note,
          });
          setIsDayModalOpen(true);
          return;
        }
      }
    } catch (e) {
      console.error('Error fetching day log:', e);
    }

    setSelectedDay({
      date,
      day_type: dayType,
    });
    setIsDayModalOpen(true);
  };

  const handleLogToday = () => {
    const todayStr = new Date().toISOString().split('T')[0];
    handleSelectDate(todayStr);
  };

  const profile = profileData?.profile || null;
  const stats = useMemo<LifeProfileStats | null>(() => {
    if (!profile || !profileData) return null;
    const ageInDays = profileData.age_days ?? profileData.stats?.age_in_days ?? 0;
    const totalDays =
      profileData.total_expected_days ??
      profileData.stats?.total_life_days ??
      Math.floor((profile.expected_lifespan_years || 80) * 365.25);
    const lifeLived =
      profileData.days_lived_percent ??
      profileData.stats?.life_lived_percentage ??
      (totalDays > 0 ? (ageInDays / totalDays) * 100 : 0);
    const loggedCount = profileData.total_logged_days ?? profileData.stats?.days_logged ?? 0;
    const dominantMood = profileData.dominant_mood ?? profileData.stats?.dominant_mood ?? null;

    return {
      birth_date: profile.birth_date,
      expected_lifespan_years: profile.expected_lifespan_years,
      age_in_days: ageInDays,
      total_life_days: totalDays,
      life_lived_percentage: lifeLived,
      days_logged: loggedCount,
      dominant_mood: dominantMood,
    };
  }, [profileData, profile]);

  const allLifespanDays = useMemo<LifeHorizonDay[]>(() => {
    if (!profile?.birth_date) return [];
    const birthParts = profile.birth_date.split('-').map(Number);
    if (birthParts.length < 3 || isNaN(birthParts[0])) return [];
    const birthYear = birthParts[0];
    const lifespan = profile.expected_lifespan_years || 80;
    const endYear = birthYear + lifespan - 1;
    const todayStr = new Date().toISOString().split('T')[0];

    const loggedMap = new Map<string, string>();
    for (const d of horizonDays) {
      if (d.date && d.mood) loggedMap.set(d.date, d.mood);
    }

    const result: LifeHorizonDay[] = [];
    let idx = 0;
    for (let yr = birthYear; yr <= endYear; yr++) {
      const isLeap = (yr % 4 === 0 && yr % 100 !== 0) || (yr % 400 === 0);
      const daysInYear = isLeap ? 366 : 365;
      const curDate = new Date(Date.UTC(yr, 0, 1));
      for (let d = 0; d < daysInYear; d++) {
        const dateStr = curDate.toISOString().split('T')[0];
        let status: 'past_empty' | 'past_logged' | 'present' | 'future' = 'past_empty';
        const loggedMood = loggedMap.get(dateStr) || null;
        if (dateStr === todayStr) {
          status = 'present';
        } else if (dateStr > todayStr) {
          status = 'future';
        } else if (loggedMood) {
          status = 'past_logged';
        }
        result.push({
          index: idx++,
          year: yr,
          date: dateStr,
          status,
          mood: loggedMood,
        });
        curDate.setUTCDate(curDate.getUTCDate() + 1);
      }
    }
    return result;
  }, [profile, horizonDays]);

  return (
    <div className="space-y-5 pb-8">
      {/* Top Header */}
      <div className="flex flex-col sm:flex-row sm:items-baseline justify-between gap-3">
        <div>
          <h1 className="text-xl font-bold tracking-tight text-zinc-900 dark:text-zinc-100 flex items-center gap-2">
            <Hourglass className="w-5 h-5 text-zinc-700 dark:text-zinc-300" />
            <span>Visualizing Life</span>
          </h1>
          <p className="text-xs text-zinc-500 dark:text-zinc-400 mt-0.5">
            Every block represents one day of your life. Future is white, today is the beacon, past holds your memories.
          </p>
        </div>

        <div className="flex items-center gap-2">
          {profile && (
            <button
              onClick={handleLogToday}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium text-white bg-zinc-900 hover:bg-zinc-800 dark:bg-zinc-100 dark:text-zinc-900 dark:hover:bg-white rounded-md shadow-xs transition-colors"
            >
              <Plus className="w-3.5 h-3.5" />
              <span>Log today</span>
            </button>
          )}

          <button
            onClick={() => setIsProfileModalOpen(true)}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900 text-zinc-700 dark:text-zinc-300 hover:bg-zinc-50 dark:hover:bg-zinc-800 rounded-md transition-colors"
          >
            <Calendar className="w-3.5 h-3.5 text-zinc-500" />
            <span>{profile ? 'Edit profile' : 'Set birth date'}</span>
          </button>
        </div>
      </div>

      {/* Onboarding Banner if no profile exists */}
      {!isLoadingProfile && !profile && (
        <div className="p-5 rounded-lg bg-zinc-50 dark:bg-zinc-900/60 border border-zinc-200 dark:border-zinc-800 flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4">
          <div className="space-y-0.5">
            <h3 className="text-sm font-semibold text-zinc-900 dark:text-zinc-100">
              Start visualizing your lifespan
            </h3>
            <p className="text-xs text-zinc-500 dark:text-zinc-400 max-w-xl leading-relaxed">
              Set your birth date to unlock your personal 80-year Life Horizon and lifespan grid of all days.
              Every block is a canvas for your mood, reflections, and life milestones.
            </p>
          </div>
          <button
            onClick={() => setIsProfileModalOpen(true)}
            className="px-3.5 py-1.5 rounded-md bg-zinc-900 text-white hover:bg-zinc-800 dark:bg-zinc-100 dark:text-zinc-900 dark:hover:bg-white font-medium text-xs shadow-xs transition-colors shrink-0"
          >
            Set birth date now
          </button>
        </div>
      )}

      {/* 1. Life Statistics Strip (Strictly no age references) */}
      {stats && (
        <LifeStatStrip
          stats={stats}
          onOpenSettings={() => setIsProfileModalOpen(true)}
        />
      )}

      {/* 2. Macro Life Horizon Overview (~30,000 days canvas) */}
      {profile && allLifespanDays.length > 0 && (
        <LifeHorizonMinimap
          days={allLifespanDays}
          currentYear={currentActualYear}
          onSelectYear={(year) => {
            const el = document.getElementById(`year-${year}`);
            if (el) el.scrollIntoView({ behavior: 'smooth', block: 'center' });
          }}
          isDark={isDark}
        />
      )}

      {/* 3. All Blocks Lifespan Grid (No calendar) */}
      {profile && allLifespanDays.length > 0 && (
        <LifeAllBlocksGrid
          days={allLifespanDays}
          onSelectDate={handleSelectDate}
          selectedDate={selectedDate}
        />
      )}

      {/* Day Logging Modal */}
      <LifeDayModal
        day={selectedDay}
        isOpen={isDayModalOpen}
        onClose={() => {
          setIsDayModalOpen(false);
          setSelectedDay(null);
        }}
        onSave={async (payload) => {
          await saveDayMutation.mutateAsync(payload);
        }}
        onDelete={async (date) => {
          await deleteDayMutation.mutateAsync(date);
        }}
      />

      {/* Profile / Onboarding Modal */}
      <LifeProfileModal
        profile={profile}
        isOpen={isProfileModalOpen}
        onClose={() => setIsProfileModalOpen(false)}
        onSave={async (payload) => {
          await saveProfileMutation.mutateAsync(payload);
        }}
      />
    </div>
  );
};

export const lifeModule: FrontendModule = {
  id: 'life',
  name: 'Life Grid',
  description: 'Lifespan Day-by-Day Visualizer and Daily Reflection Grid',
  iconName: 'Hourglass',
  navTab: {
    label: 'Life Grid',
  },
  MainView: LifeView,
  handleSseEvent: (event: SystemEvent, queryClient) => {
    if (
      event.type === 'LifeProfileUpdated' ||
      event.type === 'LifeDayLogged' ||
      event.type === 'LifeDayCleared'
    ) {
      queryClient.invalidateQueries({ queryKey: ['life-profile'] });
      queryClient.invalidateQueries({ queryKey: ['life-horizon'] });
    }
  },
};
