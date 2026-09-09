import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from './apiClient';
import { useSession } from './session';

export interface UserSettings {
  /** IANA zone name, e.g. `Asia/Shanghai`. */
  timezone: string;
  /** ISO 4217 code, e.g. `CNY`. */
  currency: string;
  /** Matrix room ID where development errors and alerts are routed. */
  dev_room_id?: string | null;
}

/** Matches the server's own fallback, so an unauthenticated or still-loading UI
 *  renders the same values a fresh account would get. */
export const DEFAULT_SETTINGS: UserSettings = {
  timezone: 'UTC',
  currency: 'USD',
  dev_room_id: null,
};

export const SETTINGS_QUERY_KEY = ['settings'] as const;

/**
 * The currencies the pickers offer.
 *
 * Lives in core rather than in the finance module because the preferences
 * dialog and the ledger form must offer the same list. The API accepts any ISO
 * 4217 code, so this is a convenience shortlist, not a constraint.
 */
export const CURRENCY_OPTIONS = [
  { value: 'CNY', label: 'CNY — Chinese yuan' },
  { value: 'USD', label: 'USD — US dollar' },
  { value: 'EUR', label: 'EUR — Euro' },
  { value: 'GBP', label: 'GBP — Pound sterling' },
  { value: 'JPY', label: 'JPY — Japanese yen' },
  { value: 'HKD', label: 'HKD — Hong Kong dollar' },
  { value: 'SGD', label: 'SGD — Singapore dollar' },
  { value: 'AUD', label: 'AUD — Australian dollar' },
  { value: 'CAD', label: 'CAD — Canadian dollar' },
  { value: 'CHF', label: 'CHF — Swiss franc' },
  { value: 'INR', label: 'INR — Indian rupee' },
];

/**
 * The caller's saved preferences.
 *
 * Every module calls this rather than threading settings down through props —
 * React Query dedupes to one request and one cache entry, so a settings change
 * reformats the whole app from a single invalidation.
 */
export function useSettings(): UserSettings {
  const { isSignedIn } = useSession();
  const { data } = useQuery<UserSettings>({
    queryKey: SETTINGS_QUERY_KEY,
    queryFn: () => fetchApi<UserSettings>('/settings'),
    enabled: isSignedIn,
    // Preferences change rarely and the SSE stream announces it when they do.
    staleTime: 5 * 60 * 1000,
  });

  return data ?? DEFAULT_SETTINGS;
}

export function useUpdateSettings() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (settings: UserSettings) =>
      fetchApi<UserSettings>('/settings', {
        method: 'PUT',
        body: JSON.stringify(settings),
      }),
    onSuccess: (saved) => {
      // Seed the cache from the response rather than refetching: the server
      // normalises the currency, so this is the canonical value.
      queryClient.setQueryData(SETTINGS_QUERY_KEY, saved);
    },
  });
}

/**
 * The IANA zone names this browser knows, for the picker.
 *
 * `Intl.supportedValuesOf` is the browser's own copy of the tzdata name list —
 * the same names `chrono-tz` validates against on the server. Older engines
 * lack it, so a short fallback keeps the picker usable rather than empty.
 */
export function supportedTimezones(): string[] {
  const supported = (
    Intl as typeof Intl & { supportedValuesOf?: (key: string) => string[] }
  ).supportedValuesOf;

  if (typeof supported === 'function') {
    try {
      return supported('timeZone');
    } catch {
      /* fall through to the short list */
    }
  }

  return [
    'UTC',
    'America/Los_Angeles',
    'America/New_York',
    'Europe/London',
    'Europe/Berlin',
    'Asia/Dubai',
    'Asia/Kolkata',
    'Asia/Shanghai',
    'Asia/Singapore',
    'Asia/Tokyo',
    'Australia/Sydney',
  ];
}

/** The zone this browser is set to, offered as the obvious first choice. */
export function browserTimezone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
}

/** The current UTC offset of a zone, e.g. `UTC+08:00`, for the picker labels. */
export function offsetLabel(timezone: string): string {
  try {
    const parts = new Intl.DateTimeFormat('en-US', {
      timeZone: timezone,
      timeZoneName: 'longOffset',
    }).formatToParts(new Date());
    // `longOffset` renders UTC itself as a bare "GMT"; name it consistently.
    const name = parts.find((p) => p.type === 'timeZoneName')?.value ?? '';
    return name === 'GMT' ? 'UTC+00:00' : name.replace('GMT', 'UTC');
  } catch {
    return '';
  }
}

/**
 * Renders a stored UTC instant in the user's chosen zone.
 *
 * Passing `timeZone` explicitly is the whole point: the default would be the
 * browser's zone, which is precisely what the setting exists to override.
 */
export function formatInZone(
  iso: string,
  timezone: string,
  options: Intl.DateTimeFormatOptions
): string {
  if (!iso) return '';
  const trimmed = iso.trim();
  const normalized = trimmed.includes('Z') || /[+-]\d{2}(?::?\d{2})?$/.test(trimmed)
    ? trimmed
    : trimmed.includes('T')
      ? `${trimmed}Z`
      : `${trimmed.replace(' ', 'T')}Z`;

  const date = new Date(normalized);
  const validDate = Number.isNaN(date.getTime()) ? new Date(iso) : date;
  if (Number.isNaN(validDate.getTime())) return '';

  try {
    return validDate.toLocaleString(undefined, { ...options, timeZone: timezone });
  } catch {
    // A zone the browser rejects must not blank the timestamp entirely.
    return validDate.toLocaleString(undefined, options);
  }
}

/** Date and time, e.g. `9 Aug 2026, 18:30`. */
export function formatDateTimeInZone(iso: string, timezone: string): string {
  return formatInZone(iso, timezone, { dateStyle: 'medium', timeStyle: 'short' });
}

/** Date only, e.g. `9 Aug 2026`. */
export function formatDateInZone(iso: string, timezone: string): string {
  return formatInZone(iso, timezone, { dateStyle: 'medium' });
}

/** Time only, e.g. `18:30:00`. */
export function formatTimeInZone(iso: string, timezone: string): string {
  return formatInZone(iso, timezone, { timeStyle: 'medium' });
}

/** Short time only, e.g. `10:30 AM`. */
export function formatShortTimeInZone(iso: string, timezone: string): string {
  return formatInZone(iso, timezone, { hour: '2-digit', minute: '2-digit' });
}

const pad = (n: number) => String(n).padStart(2, '0');

/** A zone's wall-clock fields at a given instant. */
function wallClockParts(date: Date, timezone: string) {
  const parts = new Intl.DateTimeFormat('en-US', {
    timeZone: timezone,
    hourCycle: 'h23',
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  }).formatToParts(date);

  const field = (type: string) => Number(parts.find((p) => p.type === type)?.value ?? 0);

  return {
    year: field('year'),
    month: field('month'),
    day: field('day'),
    // Some engines render midnight as hour 24 rather than 0.
    hour: field('hour') % 24,
    minute: field('minute'),
    second: field('second'),
  };
}

/** A zone's UTC offset, in milliseconds, at a given instant. */
function zoneOffsetMs(date: Date, timezone: string): number {
  const p = wallClockParts(date, timezone);
  const asIfUtc = Date.UTC(p.year, p.month - 1, p.day, p.hour, p.minute, p.second);
  return asIfUtc - (date.getTime() - date.getMilliseconds());
}

/**
 * Reads a `DateTimePicker` value (`YYYY-MM-DDTHH:mm`, wall clock in the user's
 * chosen zone) as the UTC instant the API stores.
 *
 * The naive string names a wall clock, not an instant, so it has to be resolved
 * against the offset that zone was actually on at that moment. The offset is
 * applied twice because the first guess can land on the wrong side of a
 * daylight-saving transition, which would put the result an hour out.
 */
export function zonedInputToUtcIso(value: string, timezone: string): string | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})$/.exec(value.trim());
  if (!match) return null;

  const [year, month, day, hour, minute] = match.slice(1).map(Number);
  const asIfUtc = Date.UTC(year, month - 1, day, hour, minute);

  let instant = asIfUtc - zoneOffsetMs(new Date(asIfUtc), timezone);
  instant = asIfUtc - zoneOffsetMs(new Date(instant), timezone);

  const resolved = new Date(instant);
  return Number.isNaN(resolved.getTime()) ? null : resolved.toISOString();
}

/** The inverse of [`zonedInputToUtcIso`]. */
export function utcIsoToZonedInput(
  iso: string | null | undefined,
  timezone: string
): string {
  if (!iso) return '';
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return '';

  const p = wallClockParts(date, timezone);
  return `${p.year}-${pad(p.month)}-${pad(p.day)}T${pad(p.hour)}:${pad(p.minute)}`;
}

/**
 * "Now" as a naive `Date` carrying the chosen zone's wall clock.
 *
 * For calendar widgets that work in naive wall-clock terms: they compare and
 * render date fields, so they need today's date *there*, not here.
 */
export function nowInZone(timezone: string): Date {
  const p = wallClockParts(new Date(), timezone);
  return new Date(p.year, p.month - 1, p.day, p.hour, p.minute, p.second);
}
