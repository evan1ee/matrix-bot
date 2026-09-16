import React, { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../../core/apiClient';
import {
  FinanceAnalyticsReport,
  FinanceShare,
  PublicFinanceReport,
} from '../types';
import { MonthlyCashflowBarChart } from './MonthlyCashflowBarChart';
import { ExpenseCategoryPieChart } from './ExpenseCategoryPieChart';
import { formatMinor } from '../index';
import {
  Share2,
  Copy,
  Check,
  Globe,
  Lock,
  ArrowDownRight,
  ArrowUpRight,
  ExternalLink,
} from 'lucide-react';

interface PublicTransparencyViewProps {
  /** If provided, runs in public standalone unauthenticated mode */
  publicToken?: string;
  /** Active currency in authenticated mode */
  currency?: string;
}

export const PublicTransparencyView: React.FC<PublicTransparencyViewProps> = ({
  publicToken,
  currency,
}) => {
  const queryClient = useQueryClient();
  const [months, setMonths] = useState<number>(6);
  const [copied, setCopied] = useState(false);
  const [showShareModal, setShowShareModal] = useState(false);
  const [customTitle, setCustomTitle] = useState('');

  // 1. Authenticated queries
  const { data: authReport, isLoading: isAuthLoading } = useQuery<FinanceAnalyticsReport>({
    queryKey: ['finance-analytics', currency, months],
    queryFn: () =>
      fetchApi<FinanceAnalyticsReport>(
        `/finance/analytics?months=${months}${currency ? `&currency=${currency}` : ''}`
      ),
    enabled: !publicToken,
  });

  const { data: shareConfig } = useQuery<FinanceShare | null>({
    queryKey: ['finance-share'],
    queryFn: () => fetchApi<FinanceShare | null>('/finance/share'),
    enabled: !publicToken,
  });

  // 2. Unauthenticated public query
  const { data: publicReport, isLoading: isPublicLoading, error: publicError } =
    useQuery<PublicFinanceReport>({
      queryKey: ['public-finance-report', publicToken, months],
      queryFn: async () => {
        const res = await fetch(`/api/v1/public/finance/${publicToken}?months=${months}`);
        if (!res.ok) {
          throw new Error('Public report not found or link has expired.');
        }
        return res.json();
      },
      enabled: Boolean(publicToken),
    });

  // Share mutations
  const shareMutation = useMutation({
    mutationFn: (body: { is_enabled: boolean; title?: string }) =>
      fetchApi<FinanceShare>('/finance/share', {
        method: 'POST',
        body: JSON.stringify(body),
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['finance-share'] });
    },
  });

  const disableMutation = useMutation({
    mutationFn: () => fetchApi('/finance/share', { method: 'DELETE' }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['finance-share'] });
    },
  });

  // Decide active report
  const isPublic = Boolean(publicToken);
  const isLoading = isPublic ? isPublicLoading : isAuthLoading;
  const report = isPublic ? publicReport : authReport;

  if (isLoading) {
    return (
      <div className="py-16 text-center text-xs font-mono text-neutral-500">
        Loading fiscal transparency report...
      </div>
    );
  }

  if (isPublic && publicError) {
    return (
      <div className="max-w-md mx-auto my-16 p-6 rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 text-center space-y-3">
        <Lock className="w-6 h-6 mx-auto text-neutral-400" />
        <h3 className="text-sm font-medium text-neutral-900 dark:text-neutral-100 font-sans">
          Report Unavailable
        </h3>
        <p className="text-xs text-neutral-500 font-sans">
          This fiscal report link may be disabled, expired, or invalid.
        </p>
      </div>
    );
  }

  if (!report) {
    return (
      <div className="py-12 text-center text-xs font-mono text-neutral-500">
        No fiscal records found for the requested period.
      </div>
    );
  }

  const activeCurrency = report.currency;
  const exponent = report.currency_exponent;
  const totalIncome = report.total_income_minor;
  const totalExpense = report.total_expense_minor;
  const netSavings = report.net_savings_minor;

  const origin = typeof window !== 'undefined' ? window.location.origin : '';
  const shareUrl = shareConfig?.token
    ? `${origin}?public_finance=${shareConfig.token}`
    : '';

  const handleCopyLink = () => {
    if (shareUrl) {
      navigator.clipboard.writeText(shareUrl);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  };

  return (
    <div className="space-y-6">
      {/* Editorial Header / Masthead */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 pb-4 border-b border-neutral-200 dark:border-neutral-800">
        <div className="space-y-1">
          <div className="flex items-center gap-2">
            <h2 className="text-base font-semibold tracking-tight text-neutral-900 dark:text-neutral-100 font-sans">
              {isPublic
                ? publicReport?.title || 'Public Fiscal Transparency Report'
                : 'Fiscal Transparency Dashboard'}
            </h2>
            {isPublic && (
              <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-mono font-medium bg-emerald-50 dark:bg-emerald-950/40 text-emerald-700 dark:text-emerald-300 border border-emerald-200 dark:border-emerald-800">
                <Globe className="w-3 h-3" />
                Public Read-Only
              </span>
            )}
          </div>
          <p className="text-xs text-neutral-500 font-sans">
            Auditable cashflow, monthly balances, and expenditure allocation in{' '}
            <span className="font-mono text-neutral-700 dark:text-neutral-300 font-medium">
              {activeCurrency}
            </span>
          </p>
        </div>

        {/* Period Selector & Share Actions */}
        <div className="flex items-center gap-2">
          {/* Month Range Selector */}
          <div className="inline-flex items-center bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded-md p-0.5 text-xs font-mono">
            {[3, 6, 12].map((m) => (
              <button
                key={m}
                onClick={() => setMonths(m)}
                className={`px-2.5 py-1 rounded transition whitespace-nowrap cursor-pointer ${
                  months === m
                    ? 'bg-neutral-200 dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100 font-medium'
                    : 'text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-200'
                }`}
              >
                {m}M
              </button>
            ))}
          </div>

          {!isPublic && (
            <button
              onClick={() => {
                setCustomTitle(shareConfig?.title ?? 'Public Fiscal Report');
                setShowShareModal(true);
              }}
              className="inline-flex items-center gap-1.5 h-8 px-3 rounded-md border border-neutral-200 dark:border-neutral-800 hover:bg-neutral-100 dark:hover:bg-neutral-900 text-xs font-medium text-neutral-700 dark:text-neutral-300 transition cursor-pointer"
            >
              <Share2 className="w-3.5 h-3.5" />
              <span>Share</span>
            </button>
          )}
        </div>
      </div>

      {/* Stat Strip Pattern (Design.md: grid-cols-2 sm:grid-cols-4) */}
      <div className="grid grid-cols-2 sm:grid-cols-4 gap-3" data-testid="transparency-stat-strip">
        <div className="p-4 rounded-lg bg-neutral-50/50 dark:bg-neutral-950/50 border border-neutral-200 dark:border-neutral-800 space-y-1">
          <p className="text-[10px] font-mono uppercase tracking-wider text-neutral-500">
            Total Inflow
          </p>
          <p className="text-xl font-semibold font-mono tabular-nums tracking-tight text-emerald-600 dark:text-emerald-400">
            +{formatMinor(totalIncome, exponent)}
          </p>
          <p className="text-[11px] text-neutral-400 font-mono">
            {activeCurrency} · {months} months
          </p>
        </div>

        <div className="p-4 rounded-lg bg-neutral-50/50 dark:bg-neutral-950/50 border border-neutral-200 dark:border-neutral-800 space-y-1">
          <p className="text-[10px] font-mono uppercase tracking-wider text-neutral-500">
            Total Outflow
          </p>
          <p className="text-xl font-semibold font-mono tabular-nums tracking-tight text-rose-600 dark:text-rose-400">
            −{formatMinor(totalExpense, exponent)}
          </p>
          <p className="text-[11px] text-neutral-400 font-mono">
            {activeCurrency} · {months} months
          </p>
        </div>

        <div className="p-4 rounded-lg bg-neutral-50/50 dark:bg-neutral-950/50 border border-neutral-200 dark:border-neutral-800 space-y-1">
          <p className="text-[10px] font-mono uppercase tracking-wider text-neutral-500">
            Net Fiscal Balance
          </p>
          <p
            className={`text-xl font-semibold font-mono tabular-nums tracking-tight ${
              netSavings >= 0
                ? 'text-neutral-900 dark:text-neutral-100'
                : 'text-rose-600 dark:text-rose-400'
            }`}
          >
            {netSavings >= 0 ? '+' : ''}
            {formatMinor(netSavings, exponent)}
          </p>
          <p className="text-[11px] text-neutral-400 font-mono">
            {netSavings >= 0 ? 'Surplus' : 'Deficit'}
          </p>
        </div>

        <div className="p-4 rounded-lg bg-neutral-50/50 dark:bg-neutral-950/50 border border-neutral-200 dark:border-neutral-800 space-y-1">
          <p className="text-[10px] font-mono uppercase tracking-wider text-neutral-500">
            Active Categories
          </p>
          <p className="text-xl font-semibold font-mono tabular-nums tracking-tight text-neutral-900 dark:text-neutral-100">
            {report.expense_categories.length}
          </p>
          <p className="text-[11px] text-neutral-400 font-mono">Expenditure buckets</p>
        </div>
      </div>

      {/* Visualizations Grid */}
      <div className="grid grid-cols-1 lg:grid-cols-12 gap-6">
        {/* Column Chart: Monthly Income vs Expense */}
        <div className="lg:col-span-7">
          <MonthlyCashflowBarChart
            data={report.monthly_cashflow}
            currency={activeCurrency}
            exponent={exponent}
          />
        </div>

        {/* Pie/Donut Chart: Expense Categories ONLY */}
        <div className="lg:col-span-5">
          <ExpenseCategoryPieChart
            categories={report.expense_categories}
            currency={activeCurrency}
            exponent={exponent}
            totalExpenseMinor={totalExpense}
          />
        </div>
      </div>

      {/* Public Auditable Ledger Table (shown in public mode or as evidence trail) */}
      {isPublic && publicReport && publicReport.recent_transactions.length > 0 && (
        <div className="space-y-3 pt-2">
          <div className="flex items-center justify-between">
            <h4 className="text-xs font-medium text-neutral-900 dark:text-neutral-100 font-sans">
              Recent Auditable Ledger
            </h4>
            <span className="text-[11px] font-mono text-neutral-500">
              {publicReport.recent_transactions.length} verified entries
            </span>
          </div>

          <div className="overflow-x-auto rounded-lg border border-neutral-200 dark:border-neutral-800">
            <table className="w-full text-left border-collapse text-xs">
              <thead>
                <tr className="border-b border-neutral-200 dark:border-neutral-800 bg-neutral-50/60 dark:bg-neutral-900/60">
                  <th className="py-2.5 px-3 font-medium text-neutral-600 dark:text-neutral-400 font-sans">
                    Date
                  </th>
                  <th className="py-2.5 px-3 font-medium text-neutral-600 dark:text-neutral-400 font-sans">
                    Category
                  </th>
                  <th className="py-2.5 px-3 font-medium text-neutral-600 dark:text-neutral-400 font-sans">
                    Description
                  </th>
                  <th className="py-2.5 px-3 font-medium text-neutral-600 dark:text-neutral-400 text-right font-sans">
                    Amount
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y divide-neutral-200 dark:divide-neutral-850">
                {publicReport.recent_transactions.map((tx) => {
                  const isExp = tx.direction === 'expense';
                  return (
                    <tr
                      key={tx.id}
                      className="hover:bg-neutral-50 dark:hover:bg-neutral-900/40 transition-colors"
                    >
                      <td className="py-2 px-3 font-mono text-neutral-500 whitespace-nowrap">
                        {tx.occurred_at.slice(0, 10)}
                      </td>
                      <td className="py-2 px-3">
                        <span className="px-1.5 py-0.5 rounded text-[10px] font-mono bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400 border border-neutral-200 dark:border-neutral-700">
                          {tx.category_name ?? 'Uncategorised'}
                        </span>
                      </td>
                      <td className="py-2 px-3 text-neutral-900 dark:text-neutral-100 max-w-xs truncate font-sans">
                        {tx.note || (isExp ? 'Expense' : 'Income')}
                      </td>
                      <td className="py-2 px-3 text-right font-mono tabular-nums whitespace-nowrap">
                        <span
                          className={`inline-flex items-center gap-0.5 font-medium ${
                            isExp
                              ? 'text-rose-600 dark:text-rose-400'
                              : 'text-emerald-600 dark:text-emerald-400'
                          }`}
                        >
                          {isExp ? (
                            <ArrowDownRight className="w-3.5 h-3.5 inline" />
                          ) : (
                            <ArrowUpRight className="w-3.5 h-3.5 inline" />
                          )}
                          {formatMinor(tx.amount_minor, tx.currency_exponent)} {tx.currency}
                        </span>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </div>
      )}

      {/* Share Management Modal */}
      {showShareModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/40 backdrop-blur-xs">
          <div className="w-full max-w-md bg-white dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 rounded-lg shadow-xl p-5 space-y-4">
            <div className="flex items-center justify-between pb-3 border-b border-neutral-200 dark:border-neutral-800">
              <div className="flex items-center gap-2">
                <Globe className="w-4 h-4 text-neutral-600 dark:text-neutral-400" />
                <h3 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
                  Public Transparency Link
                </h3>
              </div>
              <button
                onClick={() => setShowShareModal(false)}
                className="text-xs text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-200"
              >
                Close
              </button>
            </div>

            <p className="text-xs text-neutral-500 font-sans">
              Create a read-only public URL to share your fiscal transparency dashboard. Sensitive
              matrix identifiers and room credentials are automatically stripped.
            </p>

            <label className="flex flex-col gap-1 text-xs">
              <span className="font-medium text-neutral-700 dark:text-neutral-300">
                Report Title
              </span>
              <input
                type="text"
                value={customTitle}
                onChange={(e) => setCustomTitle(e.target.value)}
                placeholder="e.g. Open Budget Report"
                className="h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-sans"
              />
            </label>

            {shareConfig?.is_enabled && shareUrl && (
              <div className="space-y-1.5">
                <span className="text-xs font-medium text-neutral-700 dark:text-neutral-300">
                  Shareable URL
                </span>
                <div className="flex items-center gap-2">
                  <input
                    readOnly
                    value={shareUrl}
                    className="flex-1 h-9 px-2.5 rounded-md bg-neutral-100 dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-xs font-mono text-neutral-600 dark:text-neutral-300 select-all"
                  />
                  <button
                    onClick={handleCopyLink}
                    className="h-9 px-3 inline-flex items-center gap-1.5 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition cursor-pointer"
                  >
                    {copied ? (
                      <>
                        <Check className="w-3.5 h-3.5" />
                        <span>Copied</span>
                      </>
                    ) : (
                      <>
                        <Copy className="w-3.5 h-3.5" />
                        <span>Copy</span>
                      </>
                    )}
                  </button>
                </div>
              </div>
            )}

            <div className="flex items-center justify-between pt-2">
              {shareConfig?.is_enabled ? (
                <button
                  type="button"
                  onClick={() => {
                    disableMutation.mutate();
                    setShowShareModal(false);
                  }}
                  className="text-xs text-rose-600 hover:underline cursor-pointer"
                >
                  Disable Public Link
                </button>
              ) : (
                <span className="text-xs text-neutral-400">Link currently disabled</span>
              )}

              <button
                type="button"
                onClick={() => {
                  shareMutation.mutate({
                    is_enabled: true,
                    title: customTitle.trim() || undefined,
                  });
                }}
                className="h-9 px-4 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition cursor-pointer"
              >
                {shareConfig?.is_enabled ? 'Update Settings' : 'Enable Public Link'}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};
