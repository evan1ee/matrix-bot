import type { components } from '../../types/api';

export function formatSenderDisplay(senderName: string, currentDisplayName?: string | null): string {
  if (senderName.startsWith('acct_')) {
    return currentDisplayName || 'You';
  }
  return senderName;
}

// Frontend-only name; server schema is Reminder.
export type ReminderItem = components['schemas']['Reminder'];

export type ReminderListResponse = components['schemas']['ReminderListResponse'];

export type JoinedRoomInfo = components['schemas']['JoinedRoomInfo'];

/** Duration units, and how many seconds one of each is worth. */
export const UNIT_SECONDS = {
  seconds: 1,
  minutes: 60,
  hours: 3600,
  days: 86400,
} as const;

export type DurationUnit = keyof typeof UNIT_SECONDS;

/**
 * Expresses a delay in the largest unit worth at least one whole step.
 *
 * The API takes a delay from now, so the edit form has to turn the stored
 * absolute `trigger_at` back into one. The value is *rounded* rather than
 * required to divide exactly: a reminder set for "in 2 hours" and opened a
 * minute later has 7121 seconds left, and "In 2 Hours" is what the user meant,
 * where "In 7121 Secs" is technically true and unreadable. The cost is that
 * saving an untouched form can shift the fire time by less than one unit —
 * acceptable when re-deriving from "now" already moves it anyway.
 */
export function secondsToDuration(totalSeconds: number): { value: number; unit: DurationUnit } {
  const units: DurationUnit[] = ['days', 'hours', 'minutes', 'seconds'];
  for (const unit of units) {
    const size = UNIT_SECONDS[unit];
    if (totalSeconds >= size) {
      return { value: Math.max(1, Math.round(totalSeconds / size)), unit };
    }
  }
  return { value: Math.max(1, totalSeconds), unit: 'seconds' };
}
