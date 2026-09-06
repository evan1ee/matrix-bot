export type MoodType = 'amazing' | 'good' | 'neutral' | 'low' | 'rough';

export type WeatherType = 'sunny' | 'cloudy' | 'rainy' | 'snowy' | 'windy';

export type TimeCategoryId =
  | 'sleep'
  | 'deep_work'
  | 'health'
  | 'social'
  | 'leisure'
  | 'chores';

export type TimeAllocation = Array<string | null>;

export interface LifeProfile {
  id: number;
  owner_id: string;
  birth_date: string;
  expected_lifespan_years: number;
  created_at: string;
  updated_at: string;
}

export interface LifeProfileStats {
  birth_date: string;
  expected_lifespan_years: number;
  age_in_days: number;
  total_life_days: number;
  life_lived_percentage: number;
  days_logged: number;
  dominant_mood: string | null;
  total_deep_work_hours?: number;
  average_sleep_hours?: number;
}

export interface LifeProfileResponse {
  profile: LifeProfile | null;
  age_days?: number;
  total_expected_days?: number;
  days_lived_percent?: number;
  total_logged_days?: number;
  dominant_mood?: string | null;
  total_deep_work_hours?: number;
  average_sleep_hours?: number;
  today_date?: string;
  stats?: LifeProfileStats | null;
}

export interface LifeHorizonDay {
  index: number;
  year: number;
  date: string;
  status: 'past_empty' | 'past_logged' | 'present' | 'future';
  mood: string | null;
  time_allocation?: string | null;
}

export interface LifeDayItem {
  date: string;
  day_type: 'past' | 'present' | 'future';
  mood?: string | null;
  weather?: string | null;
  activities?: string[] | null;
  note?: string | null;
  time_allocation?: TimeAllocation | null;
}

export interface LifeYearView {
  year: number;
  birth_date: string | null;
  days: LifeDayItem[];
}

export interface UpsertDayPayload {
  date: string;
  log_date?: string;
  mood: string;
  weather?: string | null;
  activities?: string[] | null;
  note?: string | null;
  time_allocation?: TimeAllocation | null;
}

export interface UpsertProfilePayload {
  birth_date: string;
  expected_lifespan_years?: number;
}

export const TIME_CATEGORIES: Record<
  TimeCategoryId,
  {
    id: TimeCategoryId;
    label: string;
    emoji: string;
    description: string;
    blockClass: string;
    activeClass: string;
    badgeClass: string;
  }
> = {
  sleep: {
    id: 'sleep',
    label: 'Sleep & Rest',
    emoji: '💤',
    description: 'Sleep, nap, recovery',
    blockClass: 'bg-indigo-600/80 border-indigo-700 dark:bg-indigo-500/70 dark:border-indigo-400',
    activeClass:
      'bg-indigo-100 text-indigo-900 border-indigo-300 dark:bg-indigo-950/80 dark:text-indigo-200 dark:border-indigo-800',
    badgeClass:
      'bg-indigo-50 text-indigo-700 border-indigo-200 dark:bg-indigo-950/50 dark:text-indigo-300 dark:border-indigo-900/60',
  },
  deep_work: {
    id: 'deep_work',
    label: 'Deep Work',
    emoji: '⚡',
    description: 'Focused coding, writing, building',
    blockClass: 'bg-zinc-900 border-zinc-950 dark:bg-zinc-100 dark:border-white',
    activeClass:
      'bg-zinc-900 text-white border-zinc-900 dark:bg-zinc-100 dark:text-zinc-900 dark:border-white',
    badgeClass:
      'bg-zinc-100 text-zinc-900 border-zinc-300 dark:bg-zinc-800 dark:text-zinc-100 dark:border-zinc-700',
  },
  health: {
    id: 'health',
    label: 'Health & Body',
    emoji: '🏃',
    description: 'Exercise, meals, fitness, walks',
    blockClass: 'bg-emerald-600/80 border-emerald-700 dark:bg-emerald-500/70 dark:border-emerald-400',
    activeClass:
      'bg-emerald-100 text-emerald-900 border-emerald-300 dark:bg-emerald-950/80 dark:text-emerald-200 dark:border-emerald-800',
    badgeClass:
      'bg-emerald-50 text-emerald-700 border-emerald-200 dark:bg-emerald-950/50 dark:text-emerald-300 dark:border-emerald-900/60',
  },
  social: {
    id: 'social',
    label: 'Social & Family',
    emoji: '👥',
    description: 'Family, friends, calls, meetings',
    blockClass: 'bg-amber-500/80 border-amber-600 dark:bg-amber-500/70 dark:border-amber-400',
    activeClass:
      'bg-amber-100 text-amber-900 border-amber-300 dark:bg-amber-950/80 dark:text-amber-200 dark:border-amber-800',
    badgeClass:
      'bg-amber-50 text-amber-700 border-amber-200 dark:bg-amber-950/50 dark:text-amber-300 dark:border-amber-900/60',
  },
  leisure: {
    id: 'leisure',
    label: 'Leisure & Hobby',
    emoji: '🎮',
    description: 'Gaming, reading, entertainment',
    blockClass: 'bg-sky-500/80 border-sky-600 dark:bg-sky-500/70 dark:border-sky-400',
    activeClass:
      'bg-sky-100 text-sky-900 border-sky-300 dark:bg-sky-950/80 dark:text-sky-200 dark:border-sky-800',
    badgeClass:
      'bg-sky-50 text-sky-700 border-sky-200 dark:bg-sky-950/50 dark:text-sky-300 dark:border-sky-900/60',
  },
  chores: {
    id: 'chores',
    label: 'Chores & Admin',
    emoji: '⏳',
    description: 'Commute, errands, house tasks',
    blockClass: 'bg-zinc-400/80 border-zinc-500 dark:bg-zinc-600/70 dark:border-zinc-500',
    activeClass:
      'bg-zinc-200 text-zinc-900 border-zinc-400 dark:bg-zinc-700 dark:text-zinc-200 dark:border-zinc-600',
    badgeClass:
      'bg-zinc-100 text-zinc-700 border-zinc-200 dark:bg-zinc-800/60 dark:text-zinc-300 dark:border-zinc-700',
  },
};

export const MOOD_CONFIG: Record<
  MoodType,
  { label: string; emoji: string; color: string; bgClass: string; textClass: string; borderClass: string }
> = {
  amazing: {
    label: 'Amazing',
    emoji: '🌟',
    color: '#f59e0b',
    bgClass: 'bg-amber-400 dark:bg-amber-500',
    textClass: 'text-amber-950 dark:text-amber-100',
    borderClass: 'border-amber-500',
  },
  good: {
    label: 'Good',
    emoji: '😊',
    color: '#10b981',
    bgClass: 'bg-emerald-500 dark:bg-emerald-500',
    textClass: 'text-emerald-950 dark:text-emerald-100',
    borderClass: 'border-emerald-500',
  },
  neutral: {
    label: 'Neutral',
    emoji: '😐',
    color: '#06b6d4',
    bgClass: 'bg-cyan-500 dark:bg-cyan-500',
    textClass: 'text-cyan-950 dark:text-cyan-100',
    borderClass: 'border-cyan-500',
  },
  low: {
    label: 'Low',
    emoji: '🌧️',
    color: '#a855f7',
    bgClass: 'bg-purple-500 dark:bg-purple-500',
    textClass: 'text-purple-950 dark:text-purple-100',
    borderClass: 'border-purple-500',
  },
  rough: {
    label: 'Rough',
    emoji: '⚡',
    color: '#f43f5e',
    bgClass: 'bg-rose-500 dark:bg-rose-500',
    textClass: 'text-rose-950 dark:text-rose-100',
    borderClass: 'border-rose-500',
  },
};

export const WEATHER_CONFIG: Record<WeatherType, { label: string; emoji: string }> = {
  sunny: { label: 'Sunny', emoji: '☀️' },
  cloudy: { label: 'Cloudy', emoji: '⛅' },
  rainy: { label: 'Rainy', emoji: '🌧️' },
  snowy: { label: 'Snowy', emoji: '❄️' },
  windy: { label: 'Windy', emoji: '💨' },
};

export const ACTIVITY_OPTIONS = [
  { id: 'Workout', label: 'Workout', emoji: '🏃' },
  { id: 'Family & Friends', label: 'Family & Friends', emoji: '👥' },
  { id: 'Learning', label: 'Learning', emoji: '📚' },
  { id: 'Travel', label: 'Travel', emoji: '✈️' },
  { id: 'Deep Work', label: 'Deep Work', emoji: '💻' },
  { id: 'Rest', label: 'Rest', emoji: '🛌' },
  { id: 'Nature', label: 'Nature', emoji: '🌲' },
  { id: 'Celebration', label: 'Celebration', emoji: '🎉' },
];
