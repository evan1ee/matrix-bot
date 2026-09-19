import type { components } from '../../types/api';

export type Habit = components['schemas']['Habit'];

export type DayStatus = components['schemas']['DayStatus'];

export type HabitWithStatus = components['schemas']['HabitWithStatus'];

export type HeatmapDay = components['schemas']['HeatmapDay'];

export type HabitStatsSummary = components['schemas']['HabitStatsSummary'];

// Frontend-only name; server schema is CreateHabitRequest.
export type CreateHabitPayload = components['schemas']['CreateHabitRequest'];
