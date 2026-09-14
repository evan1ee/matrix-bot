import React, { useState, useMemo } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Smile, Calendar, Sparkles } from 'lucide-react';
import { triggerAiPrompt } from '../ai';
import { fetchApi } from '../../core/apiClient';
import type { FrontendModule, SystemEvent } from '../../core/types';
import type {
  LifeProfileResponse,
  LifeProfileStats,
  LifeHorizonDay,
  LifeDayItem,
  UpsertProfilePayload,
} from './types';
import { LifeStatStrip } from './LifeStatStrip';
import { LifeOverview } from './LifeOverview';
import { LifeHorizonMinimap } from './LifeHorizonMinimap';
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
      return fetchApi<LifeProfileResponse>('/life/profile');
    },
  });

  // 2. Fetch Macro Life Horizon Array (All ~30,000 days of life)
  const { data: horizonDays = [] } = useQuery<LifeHorizonDay[]>({
    queryKey: ['life-horizon'],
    enabled: !!profileData?.profile,
    queryFn: async () => {
      return fetchApi<LifeHorizonDay[]>('/life/horizon');
    },
  });

  // Mutations
  const saveProfileMutation = useMutation({
    mutationFn: async (payload: UpsertProfilePayload) => {
      return fetchApi('/life/profile', {
        method: 'POST',
        body: JSON.stringify(payload),
      });
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
      const log = await fetchApi<any>(`/life/days/${date}`);
      if (log) {
        let acts: string[] = [];
        if (Array.isArray(log.activities)) {
          acts = log.activities;
        } else if (typeof log.activities === 'string') {
          try {
            acts = JSON.parse(log.activities);
          } catch {
            acts = [];
          }
        }

        let parsedTimeAlloc: (string | null)[] | null = null;
        if (Array.isArray(log.time_allocation)) {
          parsedTimeAlloc = log.time_allocation;
        } else if (typeof log.time_allocation === 'string') {
          try {
            parsedTimeAlloc = JSON.parse(log.time_allocation);
          } catch {
            parsedTimeAlloc = null;
          }
        }

        setSelectedDay({
          date: log.log_date,
          day_type: dayType,
          mood: log.mood || null,
          weather: log.weather,
          activities: acts,
          note: log.note,
          time_allocation: parsedTimeAlloc,
        });
        setIsDayModalOpen(true);
        return;
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
    window.dispatchEvent(new CustomEvent('app:navigate', { detail: { moduleId: 'mood' } }));
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
      total_deep_work_hours: profileData.total_deep_work_hours ?? profileData.stats?.total_deep_work_hours ?? 0,
      average_sleep_hours: profileData.average_sleep_hours ?? profileData.stats?.average_sleep_hours ?? 0,
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

    const loggedMap = new Map<string, { mood: string | null; time_allocation?: string | null }>();
    for (const d of horizonDays) {
      if (d.date) loggedMap.set(d.date, { mood: d.mood, time_allocation: d.time_allocation });
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
        const loggedEntry = loggedMap.get(dateStr);
        const loggedMood = loggedEntry?.mood || null;
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
          time_allocation: loggedEntry?.time_allocation || null,
        });
        curDate.setUTCDate(curDate.getUTCDate() + 1);
      }
    }
    return result;
  }, [profile, horizonDays]);

  return (
    <div className="space-y-6">
      {/* Action Toolbar */}
      <div className="flex items-center justify-end gap-2">
        {profile && (
          <button
            onClick={() =>
              triggerAiPrompt(
                'Give me a high-level perspective on my life progress across habits, projects, time allocation, and milestones. Offer thoughtful reflection on intentional living.'
              )
            }
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 text-neutral-700 dark:text-neutral-300 hover:bg-neutral-50 dark:hover:bg-neutral-800 rounded-md transition-colors cursor-pointer shadow-2xs"
            title="Ask AI for life perspective"
          >
            <Sparkles className="w-3.5 h-3.5 text-neutral-500" />
            <span>AI Perspective</span>
          </button>
        )}

        {profile && (
          <button
            onClick={handleLogToday}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium text-white bg-neutral-900 hover:bg-neutral-800 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-white rounded-md shadow-xs transition-colors cursor-pointer"
          >
            <Smile className="w-3.5 h-3.5" />
            <span>Log Mood →</span>
          </button>
        )}

        <button
          onClick={() => setIsProfileModalOpen(true)}
          className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 text-neutral-700 dark:text-neutral-300 hover:bg-neutral-50 dark:hover:bg-neutral-800 rounded-md transition-colors cursor-pointer"
        >
          <Calendar className="w-3.5 h-3.5 text-neutral-500" />
          <span>{profile ? 'Edit profile' : 'Set birth date'}</span>
        </button>
      </div>

      {/* Onboarding Banner if no profile exists */}
      {!isLoadingProfile && !profile && (
        <div className="p-5 rounded-lg bg-neutral-50 dark:bg-neutral-900/60 border border-neutral-200 dark:border-neutral-800 flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4">
          <div className="space-y-0.5">
            <h3 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
              Start visualizing your lifespan
            </h3>
            <p className="text-xs text-neutral-500 dark:text-neutral-400 max-w-xl leading-relaxed">
              Set your birth date to unlock your personal 80-year Life Horizon and lifespan grid of all days.
              Every block is a canvas for your mood, reflections, and life milestones.
            </p>
          </div>
          <button
            onClick={() => setIsProfileModalOpen(true)}
            className="px-3.5 py-1.5 rounded-md bg-neutral-900 text-white hover:bg-neutral-800 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-white font-medium text-xs shadow-xs transition-colors shrink-0"
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

      {/* 2. Lifespan Overview (View Only - No logging) */}
      {profile && allLifespanDays.length > 0 && (
        <LifeOverview
          days={allLifespanDays}
          isDark={isDark}
        />
      )}

      {/* 3. Interactive Lifespan Day Grid */}
      {profile && allLifespanDays.length > 0 && (
        <LifeHorizonMinimap
          days={allLifespanDays}
          onSelectDate={handleSelectDate}
        />
      )}

      {/* Day Retrospective Modal (View Only) */}
      <LifeDayModal
        day={selectedDay}
        isOpen={isDayModalOpen}
        onClose={() => {
          setIsDayModalOpen(false);
          setSelectedDay(null);
        }}
        onOpenMood={(date) => {
          window.dispatchEvent(new CustomEvent('app:navigate', { detail: { moduleId: 'mood', date } }));
        }}
        onOpenTimeBlocks={(date) => {
          window.dispatchEvent(new CustomEvent('app:navigate', { detail: { moduleId: 'time_blocks', date } }));
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
