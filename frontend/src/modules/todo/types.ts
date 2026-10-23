import type { components } from '../../types/api';

export type TodoItem = components['schemas']['TodoItem'];
export type TodoListResponse = components['schemas']['TodoListResponse'];
export type JoinedRoomInfo = components['schemas']['JoinedRoomInfo'];
export type TodoDetail = components['schemas']['TodoDetail'];
export type TodoStep = components['schemas']['TodoStepDto'];
export type TodoTagsResponse = components['schemas']['TodoTagsResponse'];

export const RECURRENCE_OPTIONS = ['none', 'daily', 'weekly', 'monthly', 'yearly'] as const;

/** Lead-time presets, in minutes. `''` means no reminder. */
export const LEAD_OPTIONS = [
  { value: '', label: 'No reminder' },
  { value: '10', label: '10 minutes before' },
  { value: '60', label: '1 hour before' },
  { value: '1440', label: '1 day before' },
  { value: '10080', label: '1 week before' },
];

export function leadLabel(minutes: number): string {
  return LEAD_OPTIONS.find((o) => o.value === String(minutes))?.label ?? `${minutes} min before`;
}

/** The editable fields of a task, as the PATCH body expects them. */
export interface TodoPatch {
  text: string;
  priority: string;
  due_at?: string | null;
  remind_before_minutes: number | null;
  remind_room_id?: string;
  tags?: string[];
  recurrence?: string;
}

export const priorityBadgeClass = (priority: string) =>
  priority === 'high'
    ? 'bg-rose-100 dark:bg-rose-950/40 text-rose-700 dark:text-rose-300 border-rose-300 dark:border-rose-900/50'
    : priority === 'low'
    ? 'bg-neutral-100 dark:bg-neutral-900 text-neutral-600 dark:text-neutral-400 border-neutral-200 dark:border-neutral-800'
    : 'bg-amber-100 dark:bg-amber-950/40 text-amber-800 dark:text-amber-300 border-amber-300 dark:border-amber-900/50';
