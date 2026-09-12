import React, { useEffect, useMemo, useState } from 'react';
import { Check, Clock, Coins, MessageSquare, Search, Terminal, X } from 'lucide-react';
import { useQuery } from '@tanstack/react-query';
import { CustomSelect } from './CustomSelect';
import { RoomsPanel } from './RoomsPanel';
import { SecurityPanel } from './SecurityPanel';
import { fetchApi } from '../core/apiClient';
import {
  browserTimezone,
  CURRENCY_OPTIONS,
  formatDateTimeInZone,
  offsetLabel,
  supportedTimezones,
  useSettings,
  useUpdateSettings,
} from '../core/settings';

interface SettingsModalProps {
  isOpen: boolean;
  onClose: () => void;
}

interface JoinedRoom {
  room_id: string;
  name: string;
}

/** How many zones the list renders before asking the user to narrow the filter.
 *  The full tzdata list is several hundred names; rendering all of them makes
 *  the popover scroll for a very long time to no benefit. */
const VISIBLE_ZONE_LIMIT = 60;

export const SettingsModal: React.FC<SettingsModalProps> = ({ isOpen, onClose }) => {
  const saved = useSettings();
  const { mutateAsync, isPending } = useUpdateSettings();

  const [tab, setTab] = useState<'preferences' | 'rooms' | 'security'>('preferences');
  const [timezone, setTimezone] = useState(saved.timezone);
  const [currency, setCurrency] = useState(saved.currency);
  const [devRoomId, setDevRoomId] = useState(saved.dev_room_id ?? '');
  const [maxHistoryTurns, setMaxHistoryTurns] = useState(saved.max_history_turns ?? 10);
  const [filter, setFilter] = useState('');
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const { data: rooms = [] } = useQuery<JoinedRoom[]>({
    queryKey: ['rooms'],
    queryFn: () => fetchApi<JoinedRoom[]>('/rooms'),
    enabled: isOpen,
  });

  // Reopening must show what is actually saved, not whatever was left half-typed
  // when the modal was dismissed.
  useEffect(() => {
    if (isOpen) {
      setTimezone(saved.timezone);
      setCurrency(saved.currency);
      setDevRoomId(saved.dev_room_id ?? '');
      setMaxHistoryTurns(saved.max_history_turns ?? 10);
      setFilter('');
      setErrorMsg(null);
    }
  }, [isOpen, saved.timezone, saved.currency, saved.dev_room_id, saved.max_history_turns]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && isOpen) onClose();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [isOpen, onClose]);

  const zones = useMemo(() => supportedTimezones(), []);
  const detected = useMemo(() => browserTimezone(), []);

  const matches = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    const pool = needle
      ? zones.filter((z) => z.toLowerCase().includes(needle))
      : // With no filter, keep the current choice visible rather than making the
        // user scroll to find what is already selected.
        [timezone, ...zones.filter((z) => z !== timezone)];
    return pool;
  }, [zones, filter, timezone]);

  const roomOptions = useMemo(() => {
    const list = [
      { value: '', label: 'None (default / room named DEV)' },
      ...rooms.map((r) => ({
        value: r.room_id,
        label: `${r.name} (${r.room_id})`,
      })),
    ];
    if (devRoomId && !list.some((o) => o.value === devRoomId)) {
      list.push({ value: devRoomId, label: `Custom (${devRoomId})` });
    }
    return list;
  }, [rooms, devRoomId]);

  if (!isOpen) return null;

  const dirty =
    timezone !== saved.timezone ||
    currency !== saved.currency ||
    (devRoomId.trim() || null) !== (saved.dev_room_id ?? null) ||
    maxHistoryTurns !== (saved.max_history_turns ?? 10);

  const handleSave = async () => {
    setErrorMsg(null);
    try {
      await mutateAsync({
        timezone,
        currency,
        dev_room_id: devRoomId.trim() || null,
        max_history_turns: maxHistoryTurns,
      });
      onClose();
    } catch (err: unknown) {
      setErrorMsg(err instanceof Error ? err.message : 'Could not save settings');
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 bg-black/60 dark:bg-black/80 flex items-center justify-center p-4"
      role="dialog"
      aria-modal="true"
      aria-labelledby="settings-modal-title"
    >
      <div className="bg-white dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 rounded-lg max-w-lg w-full max-h-[85vh] overflow-y-auto p-6 space-y-6 shadow-2xl">
        <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-4">
          <div className="flex items-center gap-4">
            <h3 id="settings-modal-title" className="sr-only">
              Settings
            </h3>
            {(['preferences', 'rooms', 'security'] as const).map((t) => (
              <button
                key={t}
                type="button"
                onClick={() => setTab(t)}
                aria-current={tab === t}
                className={`text-sm font-semibold capitalize transition ${
                  tab === t
                    ? 'text-neutral-900 dark:text-neutral-100'
                    : 'text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-300'
                }`}
              >
                {t}
              </button>
            ))}
          </div>
          <button
            onClick={onClose}
            aria-label="Close preferences"
            className="text-neutral-400 hover:text-neutral-900 dark:hover:text-white transition"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {tab === 'security' ? (
          <SecurityPanel />
        ) : tab === 'rooms' ? (
          <RoomsPanel />
        ) : (
        <>
        {errorMsg && (
          <div className="p-3 rounded bg-rose-100 dark:bg-rose-950/40 border border-rose-200 dark:border-rose-800/50 text-rose-800 dark:text-rose-300 text-xs font-mono">
            {errorMsg}
          </div>
        )}

        {/* Timezone */}
        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <label
              htmlFor="timezone-filter"
              className="flex items-center gap-1.5 text-xs font-mono text-neutral-600 dark:text-neutral-400"
            >
              <Clock className="w-3.5 h-3.5" />
              Timezone
            </label>
            {detected !== timezone && (
              <button
                type="button"
                onClick={() => setTimezone(detected)}
                className="text-[11px] font-mono text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-200 transition underline"
              >
                Use {detected}
              </button>
            )}
          </div>

          <div className="relative">
            <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-neutral-400 pointer-events-none" />
            <input
              id="timezone-filter"
              type="text"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="Filter zones, e.g. Shanghai"
              className="w-full pl-9 pr-3 py-2 rounded bg-neutral-50 dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-600 focus:outline-none focus:border-neutral-500 font-sans"
            />
          </div>

          <div
            role="listbox"
            aria-label="Timezone"
            className="max-h-48 overflow-y-auto rounded-lg border border-neutral-200 dark:border-neutral-800 divide-y divide-neutral-100 dark:divide-neutral-900"
          >
            {matches.length === 0 && (
              <p className="px-3 py-3 text-xs font-mono text-neutral-500">No matching zone.</p>
            )}
            {matches.slice(0, VISIBLE_ZONE_LIMIT).map((zone) => (
              <button
                key={zone}
                type="button"
                role="option"
                aria-selected={zone === timezone}
                onClick={() => setTimezone(zone)}
                className={`w-full px-3 py-2 flex items-center justify-between gap-2 text-left text-xs font-mono transition cursor-pointer ${
                  zone === timezone
                    ? 'bg-neutral-100 dark:bg-neutral-900 text-neutral-900 dark:text-neutral-100 font-medium'
                    : 'text-neutral-700 dark:text-neutral-300 hover:bg-neutral-50 dark:hover:bg-neutral-900/60'
                }`}
              >
                <span className="truncate">{zone}</span>
                <span className="flex items-center gap-2 shrink-0">
                  <span className="text-[10px] text-neutral-500">{offsetLabel(zone)}</span>
                  {zone === timezone && <Check className="w-3.5 h-3.5" />}
                </span>
              </button>
            ))}
            {matches.length > VISIBLE_ZONE_LIMIT && (
              <p className="px-3 py-2 text-[11px] font-mono text-neutral-500">
                {matches.length - VISIBLE_ZONE_LIMIT} more — keep typing to narrow.
              </p>
            )}
          </div>

          <p className="text-[11px] font-mono text-neutral-500">
            Right now: {formatDateTimeInZone(new Date().toISOString(), timezone)}
          </p>
        </div>

        {/* Default currency */}
        <div className="space-y-2">
          <label className="flex items-center gap-1.5 text-xs font-mono text-neutral-600 dark:text-neutral-400">
            <Coins className="w-3.5 h-3.5" />
            Default currency
          </label>
          <CustomSelect
            value={currency}
            onChange={setCurrency}
            options={CURRENCY_OPTIONS}
            mono
            showCheckmark
            className="w-full"
          />
          <p className="text-[11px] font-mono text-neutral-500">
            Pre-selected for new entries, and used by <code>!expense add</code> when the
            command names no currency.
          </p>
        </div>

        {/* Dev / Error alert room */}
        <div className="space-y-2">
          <label className="flex items-center gap-1.5 text-xs font-mono text-neutral-600 dark:text-neutral-400">
            <Terminal className="w-3.5 h-3.5" />
            Dev / Error alert room
          </label>
          <CustomSelect
            value={devRoomId}
            onChange={setDevRoomId}
            options={roomOptions}
            mono
            showCheckmark
            className="w-full"
          />
          <p className="text-[11px] font-mono text-neutral-500">
            System errors and AI failure notices will only be sent to this room (sent once per outage incident).
          </p>
        </div>

        {/* Max AI context history */}
        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <label className="flex items-center gap-1.5 text-xs font-mono text-neutral-600 dark:text-neutral-400">
              <MessageSquare className="w-3.5 h-3.5" />
              Max AI context history
            </label>
            <span className="text-xs font-mono text-neutral-700 dark:text-neutral-300">
              {maxHistoryTurns} messages
            </span>
          </div>
          <input
            type="range"
            min="1"
            max="50"
            step="1"
            value={maxHistoryTurns}
            onChange={(e) => setMaxHistoryTurns(parseInt(e.target.value, 10))}
            className="w-full mt-1 accent-neutral-900 dark:accent-neutral-100 cursor-pointer"
          />
          <div className="flex justify-between text-[10px] font-mono text-neutral-400">
            <span>1 (Minimal)</span>
            <span>10 (Default)</span>
            <span>50 (Deep context)</span>
          </div>
          <p className="text-[11px] font-mono text-neutral-500">
            Number of recent room or conversation messages included as context when chatting or mentioning the bot.
          </p>
        </div>

        <div className="pt-2 flex items-center justify-end gap-3 border-t border-neutral-200 dark:border-neutral-800">
          <button
            type="button"
            onClick={onClose}
            className="mt-4 text-xs font-mono text-neutral-600 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200 transition"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={handleSave}
            disabled={isPending || !dirty}
            className="mt-4 px-4 py-2 rounded bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-white transition disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {isPending ? 'Saving...' : 'Save'}
          </button>
        </div>
        </>
        )}
      </div>
    </div>
  );
};
