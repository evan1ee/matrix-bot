import React, { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import { PortfolioAuditEntry } from './types';
import {
  History,
  ArrowDownLeft,
  ArrowUpRight,
  BookmarkPlus,
  Plus,
  Minus,
  ShieldCheck,
  Filter,
} from 'lucide-react';

interface ActivitySectionProps {
  refreshTrigger?: number;
}

const QUANTITY_EXPONENT = 8;

function formatQuantity(minor: number): string {
  const scale = Math.pow(10, QUANTITY_EXPONENT);
  const whole = Math.floor(minor / scale);
  const frac = minor % scale;
  if (frac === 0) return whole.toString();
  const fracStr = frac.toString().padStart(QUANTITY_EXPONENT, '0').replace(/0+$/, '');
  return `${whole}.${fracStr}`;
}

function formatAmount(minor: number, exponent: number): string {
  const scale = Math.pow(10, exponent);
  const isNegative = minor < 0;
  const absMinor = Math.abs(minor);
  const whole = Math.floor(absMinor / scale);
  const frac = (absMinor % scale).toString().padStart(exponent, '0');
  const formatted = `${whole.toLocaleString()}.${frac}`;
  return isNegative ? `-${formatted}` : formatted;
}

function formatSignedAmount(minor: number, exponent: number): string {
  if (minor === 0) return `+${formatAmount(0, exponent)}`;
  const sign = minor > 0 ? '+' : '';
  return `${sign}${formatAmount(minor, exponent)}`;
}

function formatDate(iso: string | null | undefined): string {
  if (!iso) return '—';
  try {
    const d = new Date(iso);
    return d.toLocaleString(undefined, {
      month: 'short',
      day: 'numeric',
      year: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    });
  } catch {
    return iso;
  }
}

type FilterType = 'all' | 'buy' | 'sell' | 'cash' | 'record';

export const ActivitySection: React.FC<ActivitySectionProps> = () => {
  const [activeFilter, setActiveFilter] = useState<FilterType>('all');

  const { data: entries, isLoading } = useQuery({
    queryKey: ['portfolio', 'activity'],
    queryFn: () => fetchApi<PortfolioAuditEntry[]>('/portfolio/activity?limit=100'),
  });

  const filteredEntries = (entries ?? []).filter((entry) => {
    if (activeFilter === 'all') return true;
    if (activeFilter === 'buy') return entry.action === 'buy';
    if (activeFilter === 'sell') return entry.action === 'sell';
    if (activeFilter === 'cash') return entry.action === 'deposit' || entry.action === 'withdraw';
    if (activeFilter === 'record') return entry.action === 'record';
    return true;
  });

  const getActionBadge = (action: PortfolioAuditEntry['action']) => {
    switch (action) {
      case 'deposit':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-mono font-medium bg-emerald-50 dark:bg-emerald-950/50 text-emerald-700 dark:text-emerald-400 border border-emerald-200 dark:border-emerald-800">
            <ArrowDownLeft className="w-3 h-3" />
            DEPOSIT
          </span>
        );
      case 'withdraw':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-mono font-medium bg-orange-50 dark:bg-orange-950/50 text-orange-700 dark:text-orange-400 border border-orange-200 dark:border-orange-800">
            <ArrowUpRight className="w-3 h-3" />
            WITHDRAW
          </span>
        );
      case 'buy':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-mono font-medium bg-blue-50 dark:bg-blue-950/50 text-blue-700 dark:text-blue-400 border border-blue-200 dark:border-blue-800">
            <Plus className="w-3 h-3" />
            BUY
          </span>
        );
      case 'sell':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-mono font-medium bg-purple-50 dark:bg-purple-950/50 text-purple-700 dark:text-purple-400 border border-purple-200 dark:border-purple-800">
            <Minus className="w-3 h-3" />
            SELL
          </span>
        );
      case 'record':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-mono font-medium bg-sky-50 dark:bg-sky-950/50 text-sky-700 dark:text-sky-300 border border-sky-200 dark:border-sky-800">
            <BookmarkPlus className="w-3 h-3" />
            RECORD
          </span>
        );
      default:
        return (
          <span className="px-2 py-0.5 rounded text-[11px] font-mono font-medium bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400">
            {action}
          </span>
        );
    }
  };

  return (
    <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4">
      {/* Header */}
      <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3 flex-wrap gap-3">
        <div className="flex items-center gap-2.5">
          <History className="w-4 h-4 text-neutral-500" />
          <span className="text-xs font-mono font-medium text-neutral-700 dark:text-neutral-300 uppercase tracking-wider">
            Audit Activity Log
          </span>
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium bg-neutral-200 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400">
            <ShieldCheck className="w-3 h-3 text-emerald-600 dark:text-emerald-400" />
            Immutable
          </span>
          <span className="text-xs font-mono text-neutral-500">
            ({filteredEntries.length} records)
          </span>
        </div>

        {/* Filter Pills */}
        <div className="flex items-center gap-1 bg-neutral-100 dark:bg-neutral-900 p-1 rounded-md border border-neutral-200 dark:border-neutral-800 text-xs">
          <Filter className="w-3 h-3 text-neutral-400 ml-1 mr-0.5" />
          {(['all', 'buy', 'sell', 'cash', 'record'] as FilterType[]).map((tab) => (
            <button
              key={tab}
              onClick={() => setActiveFilter(tab)}
              className={`px-2.5 py-1 rounded text-xs font-medium transition cursor-pointer ${
                activeFilter === tab
                  ? 'bg-white dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100 shadow-xs'
                  : 'text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-200'
              }`}
            >
              {tab.charAt(0).toUpperCase() + tab.slice(1)}
            </button>
          ))}
        </div>
      </div>

      {/* Content */}
      {isLoading ? (
        <div className="text-center py-8 text-neutral-500 text-xs font-mono">
          Loading audit log...
        </div>
      ) : filteredEntries.length === 0 ? (
        <div className="text-center py-8 space-y-2">
          <History className="w-6 h-6 text-neutral-400 mx-auto opacity-50" />
          <p className="text-xs font-mono text-neutral-500">
            {activeFilter === 'all'
              ? 'No operations recorded yet. Buys, sells, deposits, withdrawals, and imported positions will appear here.'
              : `No ${activeFilter} operations found.`}
          </p>
        </div>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full text-left text-xs">
            <thead>
              <tr className="border-b border-neutral-200 dark:border-neutral-800 text-neutral-500 dark:text-neutral-400 font-mono">
                <th className="pb-2 font-normal">ACTION</th>
                <th className="pb-2 font-normal">ASSET / OPERATION</th>
                <th className="pb-2 font-normal text-right">EXECUTION PRICE</th>
                <th className="pb-2 font-normal text-right">TOTAL AMOUNT</th>
                <th className="pb-2 font-normal text-right">REALIZED P&L</th>
                <th className="pb-2 font-normal">NOTE / MEMO</th>
                <th className="pb-2 font-normal text-right">TIMESTAMP</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-neutral-200 dark:divide-neutral-800/60 font-mono">
              {filteredEntries.map((entry) => (
                <tr
                  key={entry.id}
                  className="hover:bg-neutral-100/50 dark:hover:bg-neutral-900/50 transition-colors"
                >
                  {/* Action Badge */}
                  <td className="py-2.5 pr-3 align-middle">{getActionBadge(entry.action)}</td>

                  {/* Asset / Operation Details */}
                  <td className="py-2.5 pr-3 align-middle">
                    {entry.symbol ? (
                      <div className="flex items-center gap-1.5">
                        <span className="font-bold text-neutral-900 dark:text-neutral-100">
                          {entry.symbol}
                        </span>
                        {entry.quantity_minor != null && (
                          <span className="text-neutral-500">
                            × {formatQuantity(entry.quantity_minor)}
                          </span>
                        )}
                      </div>
                    ) : (
                      <span className="text-neutral-600 dark:text-neutral-400 capitalize">
                        Cash {entry.action}
                      </span>
                    )}
                  </td>

                  {/* Execution Price */}
                  <td className="py-2.5 px-3 text-right align-middle text-neutral-600 dark:text-neutral-400">
                    {entry.price_minor != null
                      ? `${formatAmount(entry.price_minor, entry.currency_exponent)} ${entry.currency}`
                      : '—'}
                  </td>

                  {/* Total Amount / Cash Impact */}
                  <td className="py-2.5 px-3 text-right align-middle font-medium text-neutral-900 dark:text-neutral-100">
                    {entry.action === 'withdraw' ? '-' : ''}
                    {formatAmount(entry.amount_minor, entry.currency_exponent)} {entry.currency}
                  </td>

                  {/* Realized P&L */}
                  <td className="py-2.5 px-3 text-right align-middle">
                    {entry.realized_pl_minor != null ? (
                      <span
                        className={`font-semibold ${
                          entry.realized_pl_minor > 0
                            ? 'text-emerald-600 dark:text-emerald-400'
                            : entry.realized_pl_minor < 0
                            ? 'text-rose-600 dark:text-rose-400'
                            : 'text-neutral-500'
                        }`}
                      >
                        {formatSignedAmount(entry.realized_pl_minor, entry.currency_exponent)}{' '}
                        {entry.currency}
                      </span>
                    ) : (
                      <span className="text-neutral-400">—</span>
                    )}
                  </td>

                  {/* Note / Memo */}
                  <td className="py-2.5 px-3 align-middle max-w-[200px] truncate text-neutral-500 font-sans">
                    {entry.note || <span className="text-neutral-400 font-mono">—</span>}
                  </td>

                  {/* Timestamp */}
                  <td className="py-2.5 pl-3 text-right align-middle text-neutral-400 whitespace-nowrap text-[11px]">
                    {formatDate(entry.created_at)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
};
