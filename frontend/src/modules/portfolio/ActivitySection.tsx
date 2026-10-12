import React, { useState } from 'react';
import { useQuery, useMutation } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import { formatInZone, useSettings } from '../../core/settings';
import { PortfolioAuditEntry, AuditVerificationResult } from './types';
import {
  History,
  ArrowDownLeft,
  ArrowUpRight,
  BookmarkPlus,
  Plus,
  Minus,
  ShieldCheck,
  Filter,
  AlertCircle,
  Hash,
  GitMerge,
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

export function formatAmount(minor: number, exponent: number): string {
  const scale = Math.pow(10, exponent);
  const isNegative = minor < 0;
  const absMinor = Math.abs(minor);
  if (exponent <= 0) {
    const formatted = Math.round(absMinor).toLocaleString();
    return isNegative ? `-${formatted}` : formatted;
  }
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

function formatDate(iso: string | null | undefined, timezone: string): string {
  if (!iso) return '—';
  return (
    formatInZone(iso, timezone, {
      month: 'short',
      day: 'numeric',
      year: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    }) || iso
  );
}

type FilterType = 'all' | 'buy' | 'sell' | 'cash' | 'record';

export const ActivitySection: React.FC<ActivitySectionProps> = () => {
  const { timezone } = useSettings();
  const [activeFilter, setActiveFilter] = useState<FilterType>('all');
  const [verificationResult, setVerificationResult] = useState<AuditVerificationResult | null>(null);

  const { data: entries, isLoading } = useQuery({
    queryKey: ['portfolio', 'activity'],
    queryFn: () => fetchApi<PortfolioAuditEntry[]>('/portfolio/activity?limit=100'),
  });

  const verifyMutation = useMutation({
    mutationFn: () => fetchApi<AuditVerificationResult>('/portfolio/activity/verify'),
    onSuccess: (res) => {
      setVerificationResult(res);
    },
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
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-mono font-medium bg-emerald-50 dark:bg-emerald-950/40 text-emerald-700 dark:text-emerald-400 border border-emerald-200 dark:border-emerald-800">
            <ArrowDownLeft className="w-3 h-3" />
            DEPOSIT
          </span>
        );
      case 'withdraw':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-mono font-medium bg-neutral-100 dark:bg-neutral-800 text-neutral-700 dark:text-neutral-300 border border-neutral-200 dark:border-neutral-700">
            <ArrowUpRight className="w-3 h-3" />
            WITHDRAW
          </span>
        );
      case 'buy':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-mono font-medium bg-neutral-100 dark:bg-neutral-800 text-neutral-800 dark:text-neutral-200 border border-neutral-200 dark:border-neutral-700">
            <Plus className="w-3 h-3" />
            BUY
          </span>
        );
      case 'sell':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-mono font-medium bg-neutral-100 dark:bg-neutral-800 text-neutral-800 dark:text-neutral-200 border border-neutral-200 dark:border-neutral-700">
            <Minus className="w-3 h-3" />
            SELL
          </span>
        );
      case 'record':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-mono font-medium bg-neutral-100 dark:bg-neutral-800 text-neutral-800 dark:text-neutral-200 border border-neutral-200 dark:border-neutral-700">
            <BookmarkPlus className="w-3 h-3" />
            RECORD
          </span>
        );
      case 'merge':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-mono font-medium bg-neutral-100 dark:bg-neutral-800 text-neutral-800 dark:text-neutral-200 border border-neutral-200 dark:border-neutral-700">
            <GitMerge className="w-3 h-3" />
            MERGE
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
        <div className="flex items-center gap-2.5 flex-wrap">
          <History className="w-4 h-4 text-neutral-500" />
          <span className="text-xs font-mono font-medium text-neutral-700 dark:text-neutral-300 uppercase tracking-wider">
            Audit Activity Log
          </span>
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium bg-neutral-200 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400">
            <ShieldCheck className="w-3 h-3 text-emerald-600 dark:text-emerald-400" />
            SHA-256 Chained
          </span>
          <span className="text-xs font-mono text-neutral-500">
            ({filteredEntries.length} records)
          </span>

          <button
            onClick={() => verifyMutation.mutate()}
            disabled={verifyMutation.isPending}
            className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded text-xs font-mono font-medium bg-emerald-50 dark:bg-emerald-950/60 text-emerald-700 dark:text-emerald-300 border border-emerald-300 dark:border-emerald-800/80 hover:bg-emerald-100 dark:hover:bg-emerald-900/50 transition cursor-pointer disabled:opacity-50"
            title="Cryptographically verify SHA-256 hash chain from genesis to head"
          >
            <ShieldCheck className={`w-3.5 h-3.5 ${verifyMutation.isPending ? 'animate-spin' : ''}`} />
            <span>{verifyMutation.isPending ? 'Verifying...' : 'Verify Chain Integrity'}</span>
          </button>
        </div>

        {/* Filter Pills */}
        <div className="flex items-center gap-1 bg-neutral-100 dark:bg-neutral-900 p-1 rounded-md border border-neutral-200 dark:border-neutral-800 text-xs">
          <Filter className="w-3 h-3 text-neutral-400 ml-1 mr-0.5" />
          {(['all', 'buy', 'sell', 'cash', 'record'] as FilterType[]).map((tab) => (
            <button
              key={tab}
              onClick={() => setActiveFilter(tab)}
              aria-label={`Filter by ${tab}`}
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

      {/* Verification Status Banner */}
      {verificationResult && (
        <div
          className={`p-3 rounded-lg border text-xs font-mono flex items-start justify-between gap-3 ${
            verificationResult.is_valid
              ? 'bg-emerald-50/80 dark:bg-emerald-950/40 border-emerald-200 dark:border-emerald-800 text-emerald-900 dark:text-emerald-200'
              : 'bg-rose-50 dark:bg-rose-950/40 border-rose-200 dark:border-rose-800 text-rose-900 dark:text-rose-200'
          }`}
        >
          <div className="space-y-1">
            <div className="flex items-center gap-1.5 font-bold">
              {verificationResult.is_valid ? (
                <>
                  <ShieldCheck className="w-4 h-4 text-emerald-600 dark:text-emerald-400 shrink-0" />
                  <span>Cryptographic Integrity Verified: 100% Intact</span>
                </>
              ) : (
                <>
                  <AlertCircle className="w-4 h-4 text-rose-600 dark:text-rose-400 shrink-0" />
                  <span>Tampering or Chain Break Detected!</span>
                </>
              )}
            </div>
            <p className="text-[11px] opacity-90">{verificationResult.message}</p>
            {verificationResult.head_hash && (
              <p className="text-[10px] text-neutral-500 dark:text-neutral-400 truncate max-w-xl">
                Ledger Head: <span className="font-mono">{verificationResult.head_hash}</span>
              </p>
            )}
          </div>
          <button
            onClick={() => setVerificationResult(null)}
            className="text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-200 p-1 text-sm leading-none cursor-pointer"
          >
            ×
          </button>
        </div>
      )}

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
                <th className="pb-2 pr-3 font-normal">ACTION</th>
                <th className="pb-2 pr-3 font-normal">ASSET / OPERATION</th>
                <th className="pb-2 px-3 font-normal text-right">EXECUTION PRICE</th>
                <th className="pb-2 px-3 font-normal text-right">TOTAL AMOUNT</th>
                <th className="pb-2 px-3 font-normal text-right">REALIZED P&L</th>
                <th className="pb-2 px-3 font-normal">HASH (SHA-256)</th>
                <th className="pb-2 px-3 font-normal">NOTE / MEMO</th>
                <th className="pb-2 pl-3 font-normal text-right">TIMESTAMP</th>
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
                        {entry.action === 'merge' ? 'Account merge' : `Cash ${entry.action}`}
                      </span>
                    )}
                  </td>

                  {/* Execution Price */}
                  <td className="py-2.5 px-3 text-right align-middle text-neutral-600 dark:text-neutral-400">
                    {entry.action === 'merge'
                      ? '—'
                      : entry.price_minor != null
                      ? `${formatAmount(entry.price_minor, entry.currency_exponent)} ${entry.currency}`
                      : '—'}
                  </td>

                  {/* Total Amount / Cash Impact */}
                  <td className="py-2.5 px-3 text-right align-middle font-medium text-neutral-900 dark:text-neutral-100">
                    {entry.action === 'merge' ? (
                      <span className="text-neutral-400">—</span>
                    ) : (
                      <>
                        {entry.action === 'withdraw' ? '-' : ''}
                        {formatAmount(entry.amount_minor, entry.currency_exponent)} {entry.currency}
                      </>
                    )}
                  </td>

                  {/* Realized P&L */}
                  <td className="py-2.5 px-3 text-right align-middle">
                    {entry.action === 'merge' ? (
                      <span className="text-neutral-400">—</span>
                    ) : entry.realized_pl_minor != null ? (
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

                  {/* Hash Chain Fingerprint */}
                  <td className="py-2.5 px-3 align-middle">
                    <div
                      className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-mono bg-neutral-200/70 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-300 cursor-help"
                      title={`Cryptographic SHA-256 Linkage:\n• Entry Hash: ${entry.entry_hash || 'Legacy'}\n• Prev Hash: ${entry.prev_hash || 'GENESIS'}`}
                    >
                      <Hash className="w-2.5 h-2.5 opacity-50" />
                      <span>{entry.entry_hash ? entry.entry_hash.slice(0, 8) : 'legacy'}</span>
                    </div>
                  </td>

                  {/* Note / Memo */}
                  <td className="py-2.5 px-3 align-middle max-w-[200px] truncate text-neutral-500 font-sans">
                    {entry.note || <span className="text-neutral-400 font-mono">—</span>}
                  </td>

                  {/* Timestamp */}
                  <td className="py-2.5 pl-3 text-right align-middle text-neutral-400 whitespace-nowrap text-[11px]">
                    {formatDate(entry.created_at, timezone)}
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
