import React, { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  Zap,
  Plus,
  Flame,
  Search,
  CheckCircle2,
  Calendar,
  Activity,
  Sparkles,
} from 'lucide-react';
import { triggerAiPrompt } from '../ai';

import type { FrontendModule, SystemEvent } from '../../core/types';
import type { CreateHabitPayload, HabitStatsSummary, HabitWithStatus, HeatmapDay } from './types';
import { HabitHeatmap } from './HabitHeatmap';
import { HabitRow } from './HabitRow';
import { HabitModal } from './HabitModal';
import { ConfirmModal } from '../../components/ConfirmModal';

const CATEGORIES = ['all', 'wellness', 'fitness', 'learning', 'productivity', 'mindfulness'] as const;

export const HabitsView: React.FC = () => {
  const queryClient = useQueryClient();
  const [activeCategory, setActiveCategory] = useState<string>('all');
  const [searchQuery, setSearchQuery] = useState('');
  const [isModalOpen, setIsModalOpen] = useState(false);
  const [editingHabit, setEditingHabit] = useState<HabitWithStatus | null>(null);
  const [habitToDelete, setHabitToDelete] = useState<HabitWithStatus | null>(null);

  // Queries
  const { data: habits = [], isLoading: isLoadingHabits } = useQuery<HabitWithStatus[]>({
    queryKey: ['habits'],
    queryFn: async () => {
      const res = await fetch('/api/v1/habits', { credentials: 'include' });
      if (!res.ok) throw new Error('Failed to load habits');
      return res.json();
    },
  });

  const { data: heatmap = [] } = useQuery<HeatmapDay[]>({
    queryKey: ['habits-heatmap'],
    queryFn: async () => {
      const res = await fetch('/api/v1/habits/heatmap', { credentials: 'include' });
      if (!res.ok) throw new Error('Failed to load heatmap');
      return res.json();
    },
  });

  const { data: stats } = useQuery<HabitStatsSummary>({
    queryKey: ['habits-stats'],
    queryFn: async () => {
      const res = await fetch('/api/v1/habits/stats', { credentials: 'include' });
      if (!res.ok) throw new Error('Failed to load habit stats');
      return res.json();
    },
  });

  // Mutations
  const toggleMutation = useMutation({
    mutationFn: async (id: number) => {
      const res = await fetch(`/api/v1/habits/${id}/toggle`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'include',
        body: JSON.stringify({}),
      });
      if (!res.ok) throw new Error('Failed to toggle habit');
      return res.json();
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['habits'] });
      queryClient.invalidateQueries({ queryKey: ['habits-heatmap'] });
      queryClient.invalidateQueries({ queryKey: ['habits-stats'] });
    },
  });

  const saveMutation = useMutation({
    mutationFn: async ({ payload, id }: { payload: CreateHabitPayload; id?: number }) => {
      const url = id ? `/api/v1/habits/${id}` : '/api/v1/habits';
      const method = id ? 'PUT' : 'POST';
      const res = await fetch(url, {
        method,
        headers: { 'Content-Type': 'application/json' },
        credentials: 'include',
        body: JSON.stringify(payload),
      });
      if (!res.ok) {
        const data = await res.json().catch(() => ({}));
        throw new Error(data?.error || 'Failed to save habit');
      }
      return res.json();
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['habits'] });
      queryClient.invalidateQueries({ queryKey: ['habits-stats'] });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: async (id: number) => {
      const res = await fetch(`/api/v1/habits/${id}`, {
        method: 'DELETE',
        credentials: 'include',
      });
      if (!res.ok) throw new Error('Failed to delete habit');
      return res.json();
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['habits'] });
      queryClient.invalidateQueries({ queryKey: ['habits-heatmap'] });
      queryClient.invalidateQueries({ queryKey: ['habits-stats'] });
    },
  });

  // Filtered habits
  const filteredHabits = habits.filter((h) => {
    if (activeCategory !== 'all' && h.category.toLowerCase() !== activeCategory) {
      return false;
    }
    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase();
      return (
        h.name.toLowerCase().includes(q) ||
        (h.description && h.description.toLowerCase().includes(q))
      );
    }
    return true;
  });

  const completedTodayCount = habits.filter((h) => h.completed_today).length;
  const bestStreak = stats?.best_streak ?? habits.reduce((max, h) => Math.max(max, h.current_streak), 0);
  const totalCheckins = stats?.total_all_time_completions ?? habits.reduce((sum, h) => sum + h.total_completions, 0);

  return (
    <div className="space-y-6">
      {/* Vercel Stat Strip (Design.md vbg-stat-strip) */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
        <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-4">
          <span className="text-[10px] uppercase font-mono tracking-wider text-neutral-500">
            Active Habits
          </span>
          <div className="text-2xl font-semibold font-sans tabular-nums tracking-tight text-neutral-900 dark:text-neutral-100 mt-1">
            {habits.length}
          </div>
          <div className="text-[11px] text-neutral-500 font-mono mt-0.5">
            tracked daily
          </div>
        </div>

        <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-4">
          <span className="text-[10px] uppercase font-mono tracking-wider text-neutral-500">
            Today's Cadence
          </span>
          <div className="text-2xl font-semibold font-sans tabular-nums tracking-tight text-neutral-900 dark:text-neutral-100 mt-1">
            {completedTodayCount} / {habits.length}
          </div>
          <div className="text-[11px] text-neutral-500 font-mono mt-0.5">
            {habits.length > 0
              ? `${Math.round((completedTodayCount / habits.length) * 100)}% completed`
              : 'no active habits'}
          </div>
        </div>

        <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-4">
          <span className="text-[10px] uppercase font-mono tracking-wider text-neutral-500">
            Best Streak
          </span>
          <div className="text-2xl font-semibold font-sans tabular-nums tracking-tight text-neutral-900 dark:text-neutral-100 mt-1">
            {bestStreak} <span className="text-sm font-normal text-neutral-500">days</span>
          </div>
          <div className="text-[11px] text-neutral-500 font-mono mt-0.5">
            consecutive momentum
          </div>
        </div>

        <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-4">
          <span className="text-[10px] uppercase font-mono tracking-wider text-neutral-500">
            Total Check-ins
          </span>
          <div className="text-2xl font-semibold font-sans tabular-nums tracking-tight text-neutral-900 dark:text-neutral-100 mt-1">
            {totalCheckins}
          </div>
          <div className="text-[11px] text-neutral-500 font-mono mt-0.5">
            all-time completions
          </div>
        </div>
      </div>

      {/* Heatmap Card */}
      <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4">
        <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3">
          <div className="flex items-center gap-2">
            <Activity className="w-4 h-4 text-neutral-600 dark:text-neutral-300" />
            <h3 className="text-sm font-medium text-neutral-900 dark:text-neutral-200 font-sans">
              Daily Cadence Heatmap
            </h3>
          </div>
          <span className="text-xs font-mono text-neutral-500">
            365 days activity
          </span>
        </div>

        <HabitHeatmap days={heatmap} />
      </div>

      {/* Habits List Card */}
      <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4">
        {/* Header with New Habit Button */}
        <div className="flex flex-wrap items-center justify-between gap-3 border-b border-neutral-200 dark:border-neutral-800 pb-3">
          <div className="flex items-center gap-2">
            <Flame className="w-4 h-4 text-neutral-600 dark:text-neutral-300" />
            <h3 className="text-sm font-medium text-neutral-900 dark:text-neutral-200 font-sans">
              Habits & Rituals
            </h3>
            <span className="text-xs font-mono text-neutral-500">
              {filteredHabits.length} {filteredHabits.length === 1 ? 'habit' : 'habits'}
            </span>
          </div>

          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() =>
                triggerAiPrompt(
                  'Review my habit consistency streaks and completion rates. Give me personalized coaching and encouragement to build lasting momentum.'
                )
              }
              className="h-9 flex items-center gap-1.5 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-neutral-800 dark:text-neutral-200 text-xs font-medium hover:bg-neutral-100 dark:hover:bg-neutral-800 transition cursor-pointer shadow-2xs"
              title="Ask AI Habit Coach"
            >
              <Sparkles className="w-3.5 h-3.5 text-neutral-500" />
              <span>AI Coach</span>
            </button>

            <button
              type="button"
              data-testid="add-habit-button"
              onClick={() => {
                setEditingHabit(null);
                setIsModalOpen(true);
              }}
              className="h-9 flex items-center gap-1.5 px-3.5 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition cursor-pointer"
            >
              <Plus className="w-3.5 h-3.5" />
              <span>New Habit</span>
            </button>
          </div>
        </div>

        {/* Filters & Search */}
        <div className="flex flex-wrap items-center justify-between gap-3">
          {/* Category Filter Pills (matching Tasks status filter) */}
          <div className="flex items-center gap-2">
            <span className="text-[10px] uppercase font-mono tracking-wider text-neutral-500 hidden sm:inline">
              CATEGORY:
            </span>
            <div className="flex items-center bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded p-0.5 text-xs font-mono overflow-x-auto">
              {CATEGORIES.map((cat) => (
                <button
                  key={cat}
                  type="button"
                  data-testid={`category-tab-${cat}`}
                  onClick={() => setActiveCategory(cat)}
                  className={`px-2.5 py-1 rounded text-xs transition cursor-pointer uppercase ${
                    activeCategory === cat
                      ? 'bg-neutral-200 dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100 font-medium'
                      : 'text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-100'
                  }`}
                >
                  {cat}
                </button>
              ))}
            </div>
          </div>

          {/* Search Input */}
          <div className="relative flex-1 max-w-xs min-w-[200px]">
            <Search className="w-3.5 h-3.5 absolute left-2.5 top-1/2 -translate-y-1/2 text-neutral-400" />
            <input
              type="text"
              placeholder="Search habits..."
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="w-full h-9 pl-8 pr-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 focus:outline-none focus:border-neutral-500 font-sans"
            />
          </div>
        </div>

        {/* Habit List */}
        {isLoadingHabits ? (
          <div className="py-8 text-center text-xs font-mono text-neutral-400">
            Loading habits...
          </div>
        ) : filteredHabits.length === 0 ? (
          <div className="py-12 text-center space-y-2 border border-dashed border-neutral-200 dark:border-neutral-800 rounded-lg">
            <CheckCircle2 className="w-8 h-8 text-neutral-300 dark:text-neutral-700 mx-auto" />
            <p className="text-sm font-medium text-neutral-900 dark:text-neutral-100 font-sans">
              No habits found
            </p>
            <p className="text-xs text-neutral-500 font-sans max-w-sm mx-auto">
              {searchQuery || activeCategory !== 'all'
                ? 'Try clearing your filters or search term.'
                : 'Start your daily streak by clicking "New Habit" above.'}
            </p>
          </div>
        ) : (
          <div className="divide-y divide-neutral-200 dark:divide-neutral-850">
            {filteredHabits.map((habit) => (
              <HabitRow
                key={habit.id}
                habit={habit}
                onToggle={(id) => toggleMutation.mutate(id)}
                isToggling={toggleMutation.isPending}
                onEdit={(h) => {
                  setEditingHabit(h);
                  setIsModalOpen(true);
                }}
                onDelete={() => {
                  setHabitToDelete(habit);
                }}
              />
            ))}
          </div>
        )}
      </div>

      {/* Modal */}
      <HabitModal
        isOpen={isModalOpen}
        initialData={editingHabit}
        onClose={() => {
          setIsModalOpen(false);
          setEditingHabit(null);
        }}
        onSave={async (payload, id) => {
          await saveMutation.mutateAsync({ payload, id });
        }}
      />

      {/* Delete Confirmation Modal */}
      <ConfirmModal
        isOpen={!!habitToDelete}
        title="Delete Habit"
        message={
          <>
            Are you sure you want to delete{' '}
            <span className="font-semibold text-neutral-900 dark:text-neutral-100">
              "{habitToDelete?.name}"
            </span>
            ? All logged streaks and progress for this habit will be permanently removed.
          </>
        }
        confirmLabel="Delete habit"
        isDestructive
        isLoading={deleteMutation.isPending}
        onConfirm={async () => {
          if (habitToDelete) {
            await deleteMutation.mutateAsync(habitToDelete.id);
            setHabitToDelete(null);
          }
        }}
        onClose={() => setHabitToDelete(null)}
      />
    </div>
  );
};

export const habitsModule: FrontendModule = {
  id: 'habits',
  name: 'Habits',
  description: 'Daily Cadence & Heatmap Habits tracker',
  iconName: 'Zap',
  navTab: {
    label: 'Habits',
  },
  MainView: HabitsView,
  handleSseEvent: (event: SystemEvent, queryClient) => {
    if (
      event.type === 'HabitCreated' ||
      event.type === 'HabitUpdated' ||
      event.type === 'HabitDeleted' ||
      event.type === 'HabitLogToggled'
    ) {
      queryClient.invalidateQueries({ queryKey: ['habits'] });
      queryClient.invalidateQueries({ queryKey: ['habits-heatmap'] });
      queryClient.invalidateQueries({ queryKey: ['habits-stats'] });
    }
  },
};
