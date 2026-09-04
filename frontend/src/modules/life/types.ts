export type MoodType = 'amazing' | 'good' | 'neutral' | 'low' | 'rough';

export type WeatherType = 'sunny' | 'cloudy' | 'rainy' | 'snowy' | 'windy';

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
}

export interface LifeProfileResponse {
  profile: LifeProfile | null;
  stats: LifeProfileStats | null;
}

export interface LifeHorizonDay {
  index: number;
  year: number;
  date: string;
  status: 'past_empty' | 'past_logged' | 'present' | 'future';
  mood: string | null;
}

export interface LifeDayItem {
  date: string;
  day_type: 'past' | 'present' | 'future';
  mood?: string | null;
  weather?: string | null;
  activities?: string[] | null;
  note?: string | null;
}

export interface LifeYearView {
  year: number;
  birth_date: string | null;
  days: LifeDayItem[];
}

export interface UpsertDayPayload {
  date: string;
  mood: string;
  weather?: string | null;
  activities?: string[] | null;
  note?: string | null;
}

export interface UpsertProfilePayload {
  birth_date: string;
  expected_lifespan_years?: number;
}

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
