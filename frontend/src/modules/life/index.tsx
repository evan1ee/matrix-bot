import React, { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Hourglass, Sparkles, Plus, Calendar, Compass } from 'lucide-react';
import type { FrontendModule, SystemEvent } from '../../core/types';
import type {
  LifeProfileResponse,
  LifeHorizonDay,
  LifeYearView,
  LifeDayItem,
  UpsertDayPayload,
  UpsertProfilePayload,
} from './types';
import { LifeStatStrip } from './LifeStatStrip';
import { LifeHorizonMinimap } from './LifeHorizonMinimap';
import { LifeYearGrid } from './LifeYearGrid';
import { LifeDayModal } from './LifeDayModal';
import { LifeProfileModal } from './LifeProfileModal';

export const LifeView: React.FC = () => {
  const queryClient = useQueryClient();
  const currentActualYear = new Date().getFullYear();
  const [selectedYear, setSelectedYear] = useState<number>(currentActualYear);
  const [selectedDay, setSelectedDay] = useState<LifeDayItem | null>(null);
  const [isDayModalOpen, setIsDayModalOpen] = useState(false);
  const [isProfileModalOpen, setIsProfileModalOpen] = useState(false);

  // Theme check for canvas rendering
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

  // 2. Fetch Macro Life Horizon Array
  const { data: horizonDays = [] } = useQuery<LifeHorizonDay[]>({
    queryKey: ['life-horizon'],
    enabled: !!profileData?.profile,
    queryFn: async () => {
      const res = await fetch('/api/v1/life/horizon', { credentials: 'include' });
      if (!res.ok) throw new Error('Failed to load life horizon');
      return res.json();
    },
  });

  // 3. Fetch Selected Year View (~365 blocks)
  const { data: yearView, isLoading: isLoadingYear } = useQuery<LifeYearView>({
    queryKey: ['life-year', selectedYear],
    queryFn: async () => {
      const res = await fetch(`/api/v1/life/year/${selectedYear}`, { credentials: 'include' });
      if (!res.ok) throw new Error(`Failed to load year ${selectedYear}`);
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
      queryClient.invalidateQueries({ queryKey: ['life-year'] });
    },
  });

  const saveDayMutation = useMutation({
    mutationFn: async (payload: UpsertDayPayload) => {
      const res = await fetch('/api/v1/life/days', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'include',
        body: JSON.stringify(payload),
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
      queryClient.invalidateQueries({ queryKey: ['life-year', selectedYear] });
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
      queryClient.invalidateQueries({ queryKey: ['life-year', selectedYear] });
    },
  });

  const handleSelectDay = (day: LifeDayItem) => {
    setSelectedDay(day);
    setIsDayModalOpen(true);
  };

  const handleLogToday = () => {
    const todayStr = new Date().toISOString().split('T')[0];
    const todayItem =
      yearView?.days.find((d) => d.date === todayStr) || {
        date: todayStr,
        day_type: 'present' as const,
      };
    handleSelectDay(todayItem);
  };

  const profile = profileData?.profile || null;
  const stats = profileData?.stats || null;

  return (
    <div className="space-y-6 pb-8">
      {/* Top Header */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
        <div>
          <h1 className="text-xl font-bold text-zinc-900 dark:text-zinc-100 flex items-center gap-2">
            <Hourglass className="w-5 h-5 text-emerald-500" />
            <span>Visualizing Life</span>
          </h1>
          <p className="text-xs text-zinc-500 dark:text-zinc-400 mt-1">
            Every block represents one day of your life. Future is white, today is a beacon, past holds your memories.
          </p>
        </div>

        <div className="flex items-center gap-2">
          {profile && (
            <button
              onClick={handleLogToday}
              className="inline-flex items-center gap-1.5 px-3.5 py-2 text-xs font-semibold text-white bg-emerald-600 hover:bg-emerald-500 rounded-lg shadow-xs transition-colors"
            >
              <Plus className="w-4 h-4" />
              <span>Log Today</span>
            </button>
          )}

          <button
            onClick={() => setIsProfileModalOpen(true)}
            className="inline-flex items-center gap-1.5 px-3.5 py-2 text-xs font-medium border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-800 text-zinc-700 dark:text-zinc-200 hover:bg-zinc-50 dark:hover:bg-zinc-700 rounded-lg transition-colors"
          >
            <Calendar className="w-4 h-4" />
            <span>{profile ? 'Edit Profile' : 'Set Birth Date'}</span>
          </button>
        </div>
      </div>

      {/* Onboarding Banner if no profile exists */}
      {!isLoadingProfile && !profile && (
        <div className="p-6 rounded-2xl bg-gradient-to-br from-emerald-50 to-teal-50 dark:from-emerald-950/30 dark:to-zinc-900 border border-emerald-200 dark:border-emerald-800/60 shadow-sm flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4">
          <div className="space-y-1">
            <div className="flex items-center gap-2 text-emerald-800 dark:text-emerald-300 font-bold text-base">
              <Sparkles className="w-5 h-5" />
              <span>Start Visualizing Your Lifespan</span>
            </div>
            <p className="text-xs text-emerald-700 dark:text-emerald-400/90 max-w-xl leading-relaxed">
              Set your birth date to unlock your personal 80-year Life Horizon minimap and 365-day year
              calendar grid. Every day is a canvas for your mood, reflections, and life milestones.
            </p>
          </div>
          <button
            onClick={() => setIsProfileModalOpen(true)}
            className="px-4 py-2.5 rounded-xl bg-emerald-600 hover:bg-emerald-500 text-white font-semibold text-xs shadow-md transition-all shrink-0"
          >
            Set Birth Date Now
          </button>
        </div>
      )}

      {/* 1. Life Statistics Strip */}
      {stats && (
        <LifeStatStrip
          stats={stats}
          onOpenSettings={() => setIsProfileModalOpen(true)}
        />
      )}

      {/* 2. Macro Life Horizon Minimap (~30,000 days canvas) */}
      {profile && horizonDays.length > 0 && (
        <LifeHorizonMinimap
          days={horizonDays}
          currentYear={selectedYear}
          onSelectYear={setSelectedYear}
          isDark={isDark}
        />
      )}

      {/* 3. Bottom Interactive Year Calendar Grid (~365 blocks) */}
      <LifeYearGrid
        year={selectedYear}
        birthDate={profile?.birth_date || null}
        days={yearView?.days || []}
        selectedDate={selectedDay?.date || null}
        onSelectYear={setSelectedYear}
        onSelectDay={handleSelectDay}
      />

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
      queryClient.invalidateQueries({ queryKey: ['life-year'] });
    }
  },
};
