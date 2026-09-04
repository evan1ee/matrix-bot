export interface Habit {
  id: number;
  owner_id: string;
  name: string;
  description?: string | null;
  frequency: string;
  category: string;
  target_days_per_week: number;
  archived: boolean;
  created_at?: string | null;
  updated_at?: string | null;
}

export interface DayStatus {
  date: string;
  completed: boolean;
}

export interface HabitWithStatus extends Habit {
  completed_today: boolean;
  current_streak: number;
  longest_streak: number;
  total_completions: number;
  last_7_days: DayStatus[];
}

export interface HeatmapDay {
  date: string;
  count: number;
}

export interface HabitStatsSummary {
  total_active_habits: number;
  completed_today: number;
  completion_rate_today_percent: number;
  best_streak: number;
  total_all_time_completions: number;
}

export interface CreateHabitPayload {
  name: string;
  description?: string;
  category?: string;
  frequency?: string;
  target_days_per_week?: number;
}
