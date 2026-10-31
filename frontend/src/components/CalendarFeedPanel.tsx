import React, { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { CalendarDays, Check, Copy } from 'lucide-react';
import { fetchApi } from '../core/apiClient';
import { copyToClipboard } from '../core/clipboard';
import { formatDateTimeInZone, useSettings } from '../core/settings';
import { ConfirmModal } from './ConfirmModal';

interface CalendarTokenStatus {
  active: boolean;
  created_at: string | null;
}

interface CalendarTokenResponse {
  token: string;
  feed_path: string;
}

const STATUS_KEY = ['calendar-token'];

export const CalendarFeedPanel: React.FC = () => {
  const queryClient = useQueryClient();
  const settings = useSettings();
  // The plaintext link exists only right after minting; the server keeps a hash.
  const [minted, setMinted] = useState<CalendarTokenResponse | null>(null);
  const [copied, setCopied] = useState(false);
  const [confirm, setConfirm] = useState<'regenerate' | 'revoke' | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const { data: status, isLoading } = useQuery<CalendarTokenStatus>({
    queryKey: STATUS_KEY,
    queryFn: () => fetchApi<CalendarTokenStatus>('/account/calendar-token'),
  });

  const mintMutation = useMutation({
    mutationFn: () => fetchApi<CalendarTokenResponse>('/account/calendar-token', { method: 'POST' }),
    onSuccess: (data) => {
      setMinted(data);
      setErrorMsg(null);
      setConfirm(null);
      queryClient.invalidateQueries({ queryKey: STATUS_KEY });
    },
    onError: (err: unknown) => {
      setConfirm(null);
      setErrorMsg(err instanceof Error ? err.message : 'Failed to create the calendar link');
    },
  });

  const revokeMutation = useMutation({
    mutationFn: () => fetchApi('/account/calendar-token', { method: 'DELETE' }),
    onSuccess: () => {
      setMinted(null);
      setErrorMsg(null);
      setConfirm(null);
      queryClient.invalidateQueries({ queryKey: STATUS_KEY });
    },
    onError: (err: unknown) => {
      setConfirm(null);
      setErrorMsg(err instanceof Error ? err.message : 'Failed to revoke the calendar link');
    },
  });

  const feedUrl = minted ? `${window.location.origin}${minted.feed_path}` : '';
  const webcalUrl = feedUrl.replace(/^https?:/, 'webcal:');

  const handleCopy = async () => {
    if (await copyToClipboard(feedUrl)) {
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  };

  const buttonClass =
    'px-3 py-1.5 rounded text-xs font-medium border border-neutral-200 dark:border-neutral-700 text-neutral-700 dark:text-neutral-300 hover:text-neutral-900 dark:hover:text-white transition-colors duration-150 focus-visible:outline focus-visible:outline-2 focus-visible:outline-neutral-900 dark:focus-visible:outline-neutral-100';

  return (
    <div className="space-y-4">
      <div className="space-y-1">
        <h4 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100 flex items-center gap-2">
          <CalendarDays className="w-4 h-4" />
          Calendar feed
        </h4>
        <p className="text-xs text-neutral-500 dark:text-neutral-400">
          Subscribe from Apple Calendar, Google Calendar, Outlook or Thunderbird to see your dated
          todos, recurring bills and reminders. Anyone with the link can read them, so keep it
          private.
        </p>
      </div>

      {errorMsg && (
        <div
          role="alert"
          className="p-3 rounded bg-rose-100 dark:bg-rose-950/40 border border-rose-200 dark:border-rose-800/50 text-rose-800 dark:text-rose-300 text-xs font-mono"
        >
          {errorMsg}
        </div>
      )}

      {minted && (
        <div className="p-4 rounded-lg bg-neutral-50 dark:bg-neutral-900/60 border border-neutral-300 dark:border-neutral-700 space-y-3">
          <label htmlFor="calendar-feed-url" className="text-xs font-semibold text-neutral-900 dark:text-neutral-100">
            Subscription link
          </label>
          <p className="text-xs text-neutral-500 dark:text-neutral-400">
            Copy it now: it is not shown again after you close this panel.
          </p>
          <div className="flex items-center gap-2">
            <input
              id="calendar-feed-url"
              type="text"
              readOnly
              value={feedUrl}
              className="flex-1 min-w-0 bg-white dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 rounded px-3 py-1.5 text-xs font-mono text-neutral-800 dark:text-neutral-200 select-all"
            />
            <button type="button" onClick={handleCopy} className={`${buttonClass} flex items-center gap-1.5`}>
              {copied ? <Check className="w-3.5 h-3.5" /> : <Copy className="w-3.5 h-3.5" />}
              {copied ? 'Copied' : 'Copy'}
            </button>
          </div>
          <a href={webcalUrl} className={`${buttonClass} inline-block`}>
            Subscribe in calendar app
          </a>
        </div>
      )}

      {isLoading ? null : status?.active ? (
        <div className="flex items-center justify-between gap-3">
          <span className="text-xs text-neutral-500 dark:text-neutral-400">
            Link active
            {status.created_at
              ? ` since ${formatDateTimeInZone(status.created_at, settings.timezone)}`
              : ''}
          </span>
          <div className="flex items-center gap-2">
            <button type="button" onClick={() => setConfirm('regenerate')} className={buttonClass}>
              Regenerate link
            </button>
            <button type="button" onClick={() => setConfirm('revoke')} className={buttonClass}>
              Revoke link
            </button>
          </div>
        </div>
      ) : (
        <button
          type="button"
          onClick={() => mintMutation.mutate()}
          disabled={mintMutation.isPending}
          className={buttonClass}
        >
          Create calendar link
        </button>
      )}

      <ConfirmModal
        isOpen={confirm === 'regenerate'}
        onClose={() => setConfirm(null)}
        onConfirm={() => mintMutation.mutate()}
        title="Regenerate calendar link?"
        message="The current link stops working immediately. Calendars subscribed to it will need the new one."
        confirmLabel="Regenerate"
        isLoading={mintMutation.isPending}
      />
      <ConfirmModal
        isOpen={confirm === 'revoke'}
        onClose={() => setConfirm(null)}
        onConfirm={() => revokeMutation.mutate()}
        title="Revoke calendar link?"
        message="Calendars subscribed to this link will stop updating."
        confirmLabel="Revoke"
        isLoading={revokeMutation.isPending}
      />
    </div>
  );
};
