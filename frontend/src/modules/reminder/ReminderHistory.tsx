import React, { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import {
  History,
  Trash2,
  Repeat,
  RotateCcw,
  CheckCircle2,
  XCircle,
  AlertTriangle,
  ChevronLeft,
  ChevronRight,
  ChevronsLeft,
  ChevronsRight,
} from 'lucide-react';
import { CustomSelect } from '../../components/CustomSelect';
import { formatTimeInZone } from '../../core/settings';
import { useSession } from '../../core/session';
import { ReminderItem, ReminderListResponse, formatSenderDisplay } from './index';

interface ReminderHistoryProps {
  timezone: string;
  roomLabel: (roomId: string) => string;
  onReuse: (message: string, recurrence: string) => void;
}

type HistoryStatusFilter = 'history' | 'triggered' | 'cancelled' | 'failed';

const PAGE_SIZE_OPTIONS = [
  { value: '25', label: '25 / page' },
  { value: '50', label: '50 / page' },
  { value: '100', label: '100 / page' },
];

export const ReminderHistory: React.FC<ReminderHistoryProps> = ({
  timezone,
  roomLabel,
  onReuse,
}) => {
  const queryClient = useQueryClient();
  const { user } = useSession();
  const [statusFilter, setStatusFilter] = useState<HistoryStatusFilter>('history');
  const [page, setPage] = useState<number>(1);
  const [limit, setLimit] = useState<number>(25);
  const [confirmClear, setConfirmClear] = useState<boolean>(false);
  const [jumpPageInput, setJumpPageInput] = useState<string>('');

  const { data, isLoading, isPlaceholderData } = useQuery<ReminderListResponse>({
    queryKey: ['reminders-history', { status: statusFilter, page, limit }],
    queryFn: () =>
      fetchApi<ReminderListResponse>(
        `/reminders?status=${statusFilter}&page=${page}&limit=${limit}`
      ),
    placeholderData: (prev) => prev,
  });

  const total = data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / limit));

  // Reset to page 1 when filter or limit changes
  const handleFilterChange = (filter: HistoryStatusFilter) => {
    setStatusFilter(filter);
    setPage(1);
  };

  const handleLimitChange = (newLimitStr: string) => {
    const newLimit = parseInt(newLimitStr, 10) || 25;
    setLimit(newLimit);
    setPage(1);
  };

  const deleteMutation = useMutation({
    mutationFn: (id: number) =>
      fetchApi(`/reminders/${id}`, {
        method: 'DELETE',
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['reminders-history'] });
    },
  });

  const clearHistoryMutation = useMutation({
    mutationFn: () =>
      fetchApi('/reminders/history', {
        method: 'DELETE',
      }),
    onSuccess: () => {
      setConfirmClear(false);
      setPage(1);
      queryClient.invalidateQueries({ queryKey: ['reminders-history'] });
    },
  });

  const handleJumpPage = (e: React.FormEvent) => {
    e.preventDefault();
    const targetPage = parseInt(jumpPageInput, 10);
    if (!isNaN(targetPage) && targetPage >= 1 && targetPage <= totalPages) {
      setPage(targetPage);
      setJumpPageInput('');
    }
  };

  const startIdx = total === 0 ? 0 : (page - 1) * limit + 1;
  const endIdx = Math.min(page * limit, total);

  const getStatusBadge = (status: string) => {
    switch (status) {
      case 'triggered':
        return (
          <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-mono font-medium bg-emerald-50 dark:bg-emerald-950/40 text-emerald-700 dark:text-emerald-400 border border-emerald-200 dark:border-emerald-800/60">
            <CheckCircle2 className="w-3 h-3" />
            Triggered
          </span>
        );
      case 'cancelled':
        return (
          <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-mono font-medium bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400 border border-neutral-200 dark:border-neutral-700">
            <XCircle className="w-3 h-3" />
            Cancelled
          </span>
        );
      case 'failed':
        return (
          <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-mono font-medium bg-rose-50 dark:bg-rose-950/40 text-rose-700 dark:text-rose-400 border border-rose-200 dark:border-rose-800/60">
            <AlertTriangle className="w-3 h-3" />
            Failed
          </span>
        );
      default:
        return (
          <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-mono font-medium bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400 border border-neutral-200 dark:border-neutral-700">
            {status}
          </span>
        );
    }
  };

  return (
    <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4">
      {/* Header */}
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-neutral-200 dark:border-neutral-800 pb-3">
        <div className="flex items-center gap-2">
          <History className="w-4 h-4 text-neutral-600 dark:text-neutral-300" />
          <h3 className="text-sm font-medium text-neutral-900 dark:text-neutral-200">
            Reminder history
          </h3>
          <span className="text-xs font-mono text-neutral-500">
            ({total.toLocaleString()} total)
          </span>
        </div>

        <div className="flex items-center gap-3">
          {confirmClear ? (
            <div className="flex items-center gap-2">
              <span className="text-xs font-mono text-rose-600 dark:text-rose-400">
                Clear all history?
              </span>
              <button
                type="button"
                onClick={() => clearHistoryMutation.mutate()}
                disabled={clearHistoryMutation.isPending}
                className="px-2 py-1 rounded bg-rose-600 text-white text-xs font-medium hover:bg-rose-700 transition cursor-pointer disabled:opacity-50"
              >
                {clearHistoryMutation.isPending ? 'Clearing...' : 'Yes, clear'}
              </button>
              <button
                type="button"
                onClick={() => setConfirmClear(false)}
                className="px-2 py-1 rounded border border-neutral-300 dark:border-neutral-700 text-xs font-medium text-neutral-600 dark:text-neutral-400 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition cursor-pointer"
              >
                Cancel
              </button>
            </div>
          ) : (
            total > 0 && (
              <button
                type="button"
                onClick={() => setConfirmClear(true)}
                className="inline-flex items-center gap-1.5 px-2.5 py-1 text-xs font-sans font-medium text-neutral-600 dark:text-neutral-400 hover:text-rose-600 dark:hover:text-rose-400 hover:bg-neutral-200 dark:hover:bg-neutral-800 border border-neutral-200 dark:border-neutral-800 rounded transition cursor-pointer"
                title="Clear all reminder history"
              >
                <Trash2 className="w-3.5 h-3.5" />
                <span>Clear history</span>
              </button>
            )
          )}
        </div>
      </div>

      {/* Filter and Page Size Controls */}
      <div className="flex flex-wrap items-center justify-between gap-3 text-xs">
        {/* Filter Tabs */}
        <div className="flex items-center bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded p-0.5 font-mono">
          {(
            [
              { id: 'history', label: 'All' },
              { id: 'triggered', label: 'Triggered' },
              { id: 'cancelled', label: 'Cancelled' },
              { id: 'failed', label: 'Failed' },
            ] as const
          ).map((tab) => (
            <button
              key={tab.id}
              type="button"
              onClick={() => handleFilterChange(tab.id)}
              className={`px-2.5 py-1 rounded capitalize transition cursor-pointer ${
                statusFilter === tab.id
                  ? 'bg-neutral-200 dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100 font-medium'
                  : 'text-neutral-500 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200'
              }`}
            >
              {tab.label}
            </button>
          ))}
        </div>

        {/* Range & Page Size Selector */}
        <div className="flex items-center gap-3">
          <span className="text-xs font-mono text-neutral-500">
            {total > 0
              ? `Showing ${startIdx}–${endIdx} of ${total.toLocaleString()}`
              : '0 records'}
          </span>
          <div className="w-28">
            <CustomSelect
              value={String(limit)}
              onChange={handleLimitChange}
              mono
              options={PAGE_SIZE_OPTIONS}
            />
          </div>
        </div>
      </div>

      {/* History Items List */}
      {isLoading ? (
        <div className="py-8 text-center text-xs font-mono text-neutral-500">
          Loading reminder history...
        </div>
      ) : !data || data.items.length === 0 ? (
        <div className="py-8 text-center text-xs font-mono text-neutral-500 border border-dashed border-neutral-300 dark:border-neutral-800 rounded">
          No reminder history found.
        </div>
      ) : (
        <div className={`divide-y divide-neutral-200 dark:divide-neutral-850 ${isPlaceholderData ? 'opacity-60' : ''}`}>
          {data.items.map((item) => (
            <div
              key={item.id}
              className="py-3 flex items-center justify-between gap-4 transition hover:bg-neutral-100 dark:hover:bg-neutral-900/30 px-2 rounded"
            >
              <div className="space-y-1 min-w-0">
                <div className="flex flex-wrap items-center gap-2">
                  <p className="text-sm font-medium text-neutral-900 dark:text-neutral-100 line-clamp-1">
                    {item.message}
                  </p>
                  {getStatusBadge(item.status)}
                  {item.recurrence !== 'none' && (
                    <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-mono font-medium bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400 border border-neutral-200 dark:border-neutral-700">
                      <Repeat className="w-3 h-3" />
                      {item.recurrence}
                    </span>
                  )}
                </div>
                <div className="flex flex-wrap items-center gap-2 text-xs font-mono text-neutral-500">
                  <span>Trigger: {formatTimeInZone(item.trigger_at, timezone)}</span>
                  <span>•</span>
                  <span>by {formatSenderDisplay(item.sender_name, user?.display_name || user?.username)}</span>
                  <span>•</span>
                  <span className="truncate max-w-[160px]">Room: {roomLabel(item.room_id)}</span>
                  {item.retry_count > 0 && (
                    <>
                      <span>•</span>
                      <span>Retries: {item.retry_count}</span>
                    </>
                  )}
                </div>
              </div>

              <div className="flex items-center gap-2 shrink-0">
                <button
                  type="button"
                  onClick={() => onReuse(item.message, item.recurrence)}
                  className="inline-flex items-center gap-1 p-1.5 rounded text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-200 dark:hover:bg-neutral-800 transition cursor-pointer"
                  title="Re-schedule this reminder"
                >
                  <RotateCcw className="w-4 h-4" />
                </button>
                <button
                  type="button"
                  onClick={() => deleteMutation.mutate(item.id)}
                  disabled={deleteMutation.isPending}
                  className="p-1.5 rounded text-neutral-400 hover:text-rose-600 dark:hover:text-rose-400 hover:bg-neutral-200 dark:hover:bg-neutral-800 transition cursor-pointer"
                  title="Delete from history"
                >
                  <Trash2 className="w-4 h-4" />
                </button>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Pagination Controls for 10,000+ scale */}
      {totalPages > 1 && (
        <div className="pt-3 border-t border-neutral-200 dark:border-neutral-800 flex flex-wrap items-center justify-between gap-3">
          {/* Direct Page Jump Form */}
          <form onSubmit={handleJumpPage} className="flex items-center gap-2 text-xs font-mono">
            <span className="text-neutral-500">Go to page:</span>
            <input
              type="number"
              min="1"
              max={totalPages}
              value={jumpPageInput}
              onChange={(e) => setJumpPageInput(e.target.value)}
              placeholder={String(page)}
              className="w-14 h-7 px-2 rounded bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-neutral-900 dark:text-neutral-100 text-center focus:outline-none focus:border-neutral-500"
            />
            <button
              type="submit"
              className="h-7 px-2.5 rounded bg-neutral-200 dark:bg-neutral-800 hover:bg-neutral-300 dark:hover:bg-neutral-700 text-neutral-800 dark:text-neutral-200 text-xs transition cursor-pointer"
            >
              Go
            </button>
          </form>

          {/* Stepper Buttons */}
          <div className="flex items-center gap-1">
            <button
              type="button"
              onClick={() => setPage(1)}
              disabled={page <= 1}
              className="p-1.5 rounded border border-neutral-300 dark:border-neutral-800 text-neutral-600 dark:text-neutral-400 hover:bg-neutral-200 dark:hover:bg-neutral-800 disabled:opacity-40 disabled:pointer-events-none transition cursor-pointer"
              title="First page"
            >
              <ChevronsLeft className="w-3.5 h-3.5" />
            </button>
            <button
              type="button"
              onClick={() => setPage((p) => Math.max(1, p - 1))}
              disabled={page <= 1}
              className="p-1.5 rounded border border-neutral-300 dark:border-neutral-800 text-neutral-600 dark:text-neutral-400 hover:bg-neutral-200 dark:hover:bg-neutral-800 disabled:opacity-40 disabled:pointer-events-none transition cursor-pointer"
              title="Previous page"
            >
              <ChevronLeft className="w-3.5 h-3.5" />
            </button>

            <span className="px-3 text-xs font-mono text-neutral-600 dark:text-neutral-400">
              Page {page} of {totalPages}
            </span>

            <button
              type="button"
              onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
              disabled={page >= totalPages}
              className="p-1.5 rounded border border-neutral-300 dark:border-neutral-800 text-neutral-600 dark:text-neutral-400 hover:bg-neutral-200 dark:hover:bg-neutral-800 disabled:opacity-40 disabled:pointer-events-none transition cursor-pointer"
              title="Next page"
            >
              <ChevronRight className="w-3.5 h-3.5" />
            </button>
            <button
              type="button"
              onClick={() => setPage(totalPages)}
              disabled={page >= totalPages}
              className="p-1.5 rounded border border-neutral-300 dark:border-neutral-800 text-neutral-600 dark:text-neutral-400 hover:bg-neutral-200 dark:hover:bg-neutral-800 disabled:opacity-40 disabled:pointer-events-none transition cursor-pointer"
              title="Last page"
            >
              <ChevronsRight className="w-3.5 h-3.5" />
            </button>
          </div>
        </div>
      )}
    </div>
  );
};
