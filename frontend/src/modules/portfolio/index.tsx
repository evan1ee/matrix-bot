import React, { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import { FrontendModule } from '../../core/types';
import { TrendingUp } from 'lucide-react';
import { triggerAiPrompt } from '../ai';
import { useSettings } from '../../core/settings';
import { AllocationCard } from './AllocationCard';
import { PerformanceChart } from './PerformanceChart';
import { AlertsModal } from './AlertsModal';
import { ActivitySection } from './ActivitySection';
import { NetWorthStatStrip } from '../finance/components/NetWorthStatStrip';
import { CashCard } from './CashCard';
import { BuyCard } from './BuyCard';
import { HoldingTable } from './HoldingTable';
import { formatMinor, formatSigned, plColor } from './utils';
import {
  Holding,
  PortfolioCurrencySummary,
  SellResponse,
  CashBalance,
  RefreshSummary,
} from './types';

export * from './types';
export * from './utils';

export const PortfolioMainView: React.FC = () => {
  const queryClient = useQueryClient();
  const { currency: defaultCurrency, timezone } = useSettings();

  const { data: holdings, isLoading } = useQuery<Holding[]>({
    queryKey: ['portfolio-holdings'],
    queryFn: () => fetchApi<Holding[]>('/portfolio/holdings'),
  });

  const { data: summary } = useQuery<PortfolioCurrencySummary[]>({
    queryKey: ['portfolio-summary'],
    queryFn: () => fetchApi<PortfolioCurrencySummary[]>('/portfolio/summary'),
  });

  const { data: cashBalances } = useQuery<CashBalance[]>({
    queryKey: ['portfolio-cash'],
    queryFn: () => fetchApi<CashBalance[]>('/portfolio/cash'),
  });

  const invalidate = () => {
    queryClient.invalidateQueries({ queryKey: ['portfolio-holdings'] });
    queryClient.invalidateQueries({ queryKey: ['portfolio-summary'] });
    queryClient.invalidateQueries({ queryKey: ['portfolio-cash'] });
    queryClient.invalidateQueries({ queryKey: ['portfolio', 'activity'] });
    queryClient.invalidateQueries({ queryKey: ['portfolio-history'] });
    queryClient.invalidateQueries({ queryKey: ['portfolio-alerts'] });
  };

  const [refreshStatus, setRefreshStatus] = useState<string | null>(null);
  const [failedSymbols, setFailedSymbols] = useState<Set<string>>(new Set());
  const [refreshingHoldingId, setRefreshingHoldingId] = useState<number | null>(null);
  const [alertsModalOpen, setAlertsModalOpen] = useState(false);
  const [alertSymbol, setAlertSymbol] = useState('');
  const [alertCurrency, setAlertCurrency] = useState('USD');
  const [buyError, setBuyError] = useState<string | null>(null);
  const [cashError, setCashError] = useState<string | null>(null);

  const refreshPricesMutation = useMutation({
    mutationFn: () => fetchApi<RefreshSummary>('/portfolio/refresh-prices', { method: 'POST' }),
    onSuccess: (summary) => {
      invalidate();
      if (summary.failed.length > 0) {
        setFailedSymbols(new Set(summary.failed));
      } else {
        setFailedSymbols(new Set());
      }

      if (summary.updated.length > 0 && summary.failed.length > 0) {
        setRefreshStatus(
          `Updated ${summary.updated.length} holding(s). Skipped: ${summary.failed.join(', ')} (no quote found).`
        );
      } else if (summary.updated.length > 0) {
        setRefreshStatus(`Updated ${summary.updated.length} holding(s): ${summary.updated.join(', ')}`);
      } else if (summary.failed.length > 0) {
        setRefreshStatus(
          `Could not find market quotes for: ${summary.failed.join(', ')}. Check symbols or enter prices manually.`
        );
      } else {
        setRefreshStatus('No holdings to update.');
      }
      setTimeout(() => setRefreshStatus(null), 6000);
    },
    onError: (err: Error) => {
      setRefreshStatus(err.message || 'Failed to refresh prices.');
      setTimeout(() => setRefreshStatus(null), 5000);
    },
  });

  const singleRefreshMutation = useMutation({
    mutationFn: (id: number) => {
      setRefreshingHoldingId(id);
      return fetchApi<Holding>(`/portfolio/holdings/${id}/refresh-price`, { method: 'POST' });
    },
    onSuccess: (holding) => {
      invalidate();
      setFailedSymbols((prev) => {
        const next = new Set(prev);
        next.delete(holding.symbol);
        return next;
      });
      setRefreshStatus(`Price updated for ${holding.symbol}.`);
      setTimeout(() => setRefreshStatus(null), 4000);
    },
    onError: (err: Error) => {
      setRefreshStatus(err.message || 'Failed to refresh price.');
      setTimeout(() => setRefreshStatus(null), 5000);
    },
    onSettled: () => setRefreshingHoldingId(null),
  });

  const updateSymbolMutation = useMutation({
    mutationFn: ({ id, symbol }: { id: number; symbol: string }) =>
      fetchApi<Holding>(`/portfolio/holdings/${id}/symbol`, {
        method: 'PUT',
        body: JSON.stringify({ symbol }),
      }),
    onSuccess: (updated) => {
      setFailedSymbols((prev) => {
        const next = new Set(prev);
        next.delete(updated.symbol);
        return next;
      });
      invalidate();
    },
  });

  const buyMutation = useMutation({
    mutationFn: (body: Record<string, unknown>) =>
      fetchApi<Holding>('/portfolio/holdings', { method: 'POST', body: JSON.stringify(body) }),
    onSuccess: () => {
      setBuyError(null);
      invalidate();
    },
    onError: (err: Error) => setBuyError(err.message),
  });

  const recordMutation = useMutation({
    mutationFn: (body: Record<string, unknown>) =>
      fetchApi<Holding>('/portfolio/holdings/record', { method: 'POST', body: JSON.stringify(body) }),
    onSuccess: () => {
      setBuyError(null);
      invalidate();
    },
    onError: (err: Error) => setBuyError(err.message),
  });

  const sellMutation = useMutation({
    mutationFn: ({ id, ...body }: { id: number; quantity: string; price: string; executed_at?: string }) =>
      fetchApi<SellResponse>(`/portfolio/holdings/${id}/sell`, {
        method: 'POST',
        body: JSON.stringify(body),
      }),
    onSuccess: invalidate,
  });

  const priceMutation = useMutation({
    mutationFn: ({ id, price }: { id: number; price: string }) =>
      fetchApi<Holding>(`/portfolio/holdings/${id}/price`, {
        method: 'PUT',
        body: JSON.stringify({ price }),
      }),
    onSuccess: invalidate,
  });

  const noteMutation = useMutation({
    mutationFn: ({ id, note }: { id: number; note: string }) =>
      fetchApi<Holding>(`/portfolio/holdings/${id}/note`, {
        method: 'PUT',
        body: JSON.stringify({ note: note === '' ? null : note }),
      }),
    onSuccess: invalidate,
  });

  const deleteMutation = useMutation({
    mutationFn: (id: number) => fetchApi(`/portfolio/holdings/${id}`, { method: 'DELETE' }),
    onSuccess: invalidate,
  });

  const depositMutation = useMutation({
    mutationFn: (body: { amount: string; currency: string; executed_at?: string }) =>
      fetchApi<CashBalance>('/portfolio/cash/deposit', { method: 'POST', body: JSON.stringify(body) }),
    onSuccess: () => {
      setCashError(null);
      invalidate();
    },
    onError: (err: Error) => setCashError(err.message),
  });

  const withdrawMutation = useMutation({
    mutationFn: (body: { amount: string; currency: string; executed_at?: string }) =>
      fetchApi<CashBalance>('/portfolio/cash/withdraw', { method: 'POST', body: JSON.stringify(body) }),
    onSuccess: () => {
      setCashError(null);
      invalidate();
    },
    onError: (err: Error) => setCashError(err.message),
  });

  return (
    <div className="space-y-6">
      {/* Unified cross-currency net worth header widget */}
      <NetWorthStatStrip />

      {/* Per-currency summary — currencies are never added together. */}
      {summary && summary.length > 0 && (
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-4" data-testid="portfolio-summary">
          {summary.map((s) => (
            <div
              key={s.currency}
              className="p-4 rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 space-y-1"
            >
              <div className="flex items-center justify-between text-xs font-medium text-neutral-500 dark:text-neutral-400 font-sans">
                <span><span className="font-mono">{s.currency}</span> portfolio</span>
                <TrendingUp className="w-3.5 h-3.5 text-neutral-400" />
              </div>
              <div className={`text-2xl font-semibold font-sans tabular-nums tracking-tight ${plColor(s.unrealized_pl_minor)}`}>
                {formatSigned(s.unrealized_pl_minor, s.currency_exponent)}
              </div>
              <div className="text-[11px] text-neutral-500 dark:text-neutral-500 font-mono">
                value {formatMinor(s.market_value_minor, s.currency_exponent)} · cost{' '}
                {formatMinor(s.cost_basis_minor, s.currency_exponent)} · cash{' '}
                {formatMinor(s.cash_minor, s.currency_exponent)}
                {s.unpriced_holdings > 0 && (
                  <>
                    {' '}
                    · {s.unpriced_holdings} unpriced
                  </>
                )}
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Cash */}
      <CashCard
        cashBalances={cashBalances}
        defaultCurrency={defaultCurrency}
        timezone={timezone}
        isPending={depositMutation.isPending || withdrawMutation.isPending}
        onDeposit={(body) => depositMutation.mutate(body)}
        onWithdraw={(body) => withdrawMutation.mutate(body)}
        error={cashError}
      />

      {/* Add Holding */}
      <BuyCard
        defaultCurrency={defaultCurrency}
        timezone={timezone}
        isPending={buyMutation.isPending || recordMutation.isPending}
        onBuy={(body) => buyMutation.mutate(body)}
        onRecord={(body) => recordMutation.mutate(body)}
        error={buyError}
      />

      {/* Analytics & Performance Visualizations */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-5 items-stretch">
        <div className="h-full">
          <PerformanceChart
            currency={holdings?.[0]?.currency || cashBalances?.[0]?.currency || 'USD'}
            currencyExponent={holdings?.[0]?.currency_exponent ?? cashBalances?.[0]?.currency_exponent ?? 2}
          />
        </div>
        <div className="h-full">
          <AllocationCard holdings={holdings ?? []} cashBalances={cashBalances ?? []} />
        </div>
      </div>

      {/* Holdings */}
      <HoldingTable
        holdings={holdings}
        isLoading={isLoading}
        failedSymbols={failedSymbols}
        timezone={timezone}
        refreshStatus={refreshStatus}
        refreshingHoldingId={refreshingHoldingId}
        isRefreshingPrices={refreshPricesMutation.isPending}
        onRefreshPrices={() => refreshPricesMutation.mutate()}
        onSingleRefresh={(id) => singleRefreshMutation.mutate(id)}
        onSell={async (id, qty, price, executedAt) => {
          await sellMutation.mutateAsync({ id, quantity: qty, price, executed_at: executedAt });
        }}
        onSavePrice={async (id, price) => {
          await priceMutation.mutateAsync({ id, price });
        }}
        onSaveNote={async (id, note) => {
          await noteMutation.mutateAsync({ id, note });
        }}
        onSaveSymbol={async (id, symbol) => {
          await updateSymbolMutation.mutateAsync({ id, symbol });
        }}
        onDeleteHolding={async (id) => {
          await deleteMutation.mutateAsync(id);
        }}
        onOpenAlerts={(symbol, currency) => {
          setAlertSymbol(symbol);
          setAlertCurrency(currency);
          setAlertsModalOpen(true);
        }}
        onAiReview={() =>
          triggerAiPrompt(
            'Review my investment portfolio asset allocation, performance, and cash balances. Provide a concise executive overview and highlight key insights.'
          )
        }
        isSelling={sellMutation.isPending}
        isSavingPrice={priceMutation.isPending}
        isSavingNote={noteMutation.isPending}
        isSavingSymbol={updateSymbolMutation.isPending}
        isDeleting={deleteMutation.isPending}
      />

      {/* Market Price & Volatility Alerts Modal */}
      {alertsModalOpen && (
        <AlertsModal
          initialSymbol={alertSymbol}
          defaultCurrency={alertCurrency}
          onClose={() => {
            setAlertsModalOpen(false);
            setAlertSymbol('');
          }}
        />
      )}

      {/* Audit Activity Log */}
      <ActivitySection />
    </div>
  );
};

export const portfolioModule: FrontendModule = {
  id: 'portfolio',
  name: 'Portfolio',
  description: 'Track investment holdings, cost basis, and unrealized P&L per currency',
  navTab: { label: 'Portfolio' },
  MainView: PortfolioMainView,
  handleSseEvent: (event, queryClient) => {
    if (
      event.type === 'PortfolioHoldingCreated' ||
      event.type === 'PortfolioHoldingUpdated' ||
      event.type === 'PortfolioHoldingDeleted' ||
      event.type === 'PortfolioAuditLogged' ||
      event.type === 'PortfolioAlertCreated' ||
      event.type === 'PortfolioAlertTriggered' ||
      event.type === 'PortfolioAlertDeleted'
    ) {
      queryClient.invalidateQueries({ queryKey: ['portfolio-holdings'] });
      queryClient.invalidateQueries({ queryKey: ['portfolio-summary'] });
      queryClient.invalidateQueries({ queryKey: ['portfolio', 'activity'] });
      queryClient.invalidateQueries({ queryKey: ['portfolio-alerts'] });
      queryClient.invalidateQueries({ queryKey: ['portfolio-history'] });
      queryClient.invalidateQueries({ queryKey: ['net-worth'] });
    }
    if (event.type === 'PortfolioCashUpdated') {
      queryClient.invalidateQueries({ queryKey: ['portfolio-cash'] });
      queryClient.invalidateQueries({ queryKey: ['portfolio-summary'] });
      queryClient.invalidateQueries({ queryKey: ['portfolio', 'activity'] });
      queryClient.invalidateQueries({ queryKey: ['portfolio-history'] });
      queryClient.invalidateQueries({ queryKey: ['net-worth'] });
    }
  },
};
