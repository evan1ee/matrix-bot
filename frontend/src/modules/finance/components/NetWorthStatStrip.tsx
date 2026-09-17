import React from 'react';
import { useQuery } from '@tanstack/react-query';
import { fetchApi } from '../../../core/apiClient';
import { NetWorthResponse } from '../types';
import { formatMinor } from '../index';
import { TrendingUp, Wallet, Coins, RefreshCw } from 'lucide-react';

export const NetWorthStatStrip: React.FC = () => {
  const { data, isLoading, isError, refetch, isFetching } = useQuery<NetWorthResponse>({
    queryKey: ['net-worth'],
    queryFn: () => fetchApi<NetWorthResponse>('/finance/net-worth'),
  });

  if (isLoading) {
    return (
      <div className="grid grid-cols-2 sm:grid-cols-4 gap-4 animate-pulse" data-testid="net-worth-loading">
        {[...Array(4)].map((_, i) => (
          <div
            key={i}
            className="p-4 rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 space-y-2"
          >
            <div className="h-3 w-20 bg-neutral-200 dark:bg-neutral-800 rounded" />
            <div className="h-7 w-28 bg-neutral-200 dark:bg-neutral-800 rounded" />
            <div className="h-3 w-16 bg-neutral-200 dark:bg-neutral-800 rounded" />
          </div>
        ))}
      </div>
    );
  }

  if (isError || !data) {
    return null;
  }

  const hasMultipleCurrencies = data.native_breakdowns.length > 1;

  return (
    <div className="space-y-2" data-testid="net-worth-stat-strip">
      <div className="grid grid-cols-2 sm:grid-cols-4 gap-4">
        {/* Total Net Worth */}
        <div className="p-4 rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 space-y-1">
          <div className="flex items-center justify-between text-xs font-medium text-neutral-500 dark:text-neutral-400 font-sans">
            <span className="uppercase font-mono text-[10px] tracking-wider">Estimated Net Worth</span>
            <button
              type="button"
              onClick={() => refetch()}
              title="Refresh FX rates and balances"
              className="text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200 transition cursor-pointer"
            >
              <RefreshCw className={`w-3.5 h-3.5 ${isFetching ? 'animate-spin' : ''}`} />
            </button>
          </div>
          <div className="text-2xl font-semibold font-sans tabular-nums tracking-tight text-neutral-900 dark:text-neutral-100">
            {formatMinor(data.total_net_worth_minor, data.primary_currency_exponent)}
            <span className="text-xs font-mono font-normal ml-1.5 text-neutral-500 dark:text-neutral-400">
              {data.primary_currency}
            </span>
          </div>
          <div className="flex items-center gap-1.5 text-[10px] font-mono text-neutral-500 dark:text-neutral-500">
            <span className="inline-block w-1.5 h-1.5 rounded-full bg-emerald-500" />
            <span>FX: {data.fx_source}</span>
          </div>
        </div>

        {/* Ledger Cash */}
        <div className="p-4 rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 space-y-1">
          <div className="flex items-center justify-between text-xs font-medium text-neutral-500 dark:text-neutral-400 font-sans">
            <span className="uppercase font-mono text-[10px] tracking-wider">Ledger Cash</span>
            <Wallet className="w-3.5 h-3.5 text-neutral-400" />
          </div>
          <div className="text-2xl font-semibold font-sans tabular-nums tracking-tight text-neutral-900 dark:text-neutral-100">
            {formatMinor(data.ledger_cash_minor, data.primary_currency_exponent)}
            <span className="text-xs font-mono font-normal ml-1.5 text-neutral-500 dark:text-neutral-400">
              {data.primary_currency}
            </span>
          </div>
          <div className="text-[10px] font-mono text-neutral-500 dark:text-neutral-500">
            All-time balance
          </div>
        </div>

        {/* Portfolio Value */}
        <div className="p-4 rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 space-y-1">
          <div className="flex items-center justify-between text-xs font-medium text-neutral-500 dark:text-neutral-400 font-sans">
            <span className="uppercase font-mono text-[10px] tracking-wider">Investments</span>
            <TrendingUp className="w-3.5 h-3.5 text-neutral-400" />
          </div>
          <div className="text-2xl font-semibold font-sans tabular-nums tracking-tight text-neutral-900 dark:text-neutral-100">
            {formatMinor(data.portfolio_market_value_minor, data.primary_currency_exponent)}
            <span className="text-xs font-mono font-normal ml-1.5 text-neutral-500 dark:text-neutral-400">
              {data.primary_currency}
            </span>
          </div>
          <div className="text-[10px] font-mono text-neutral-500 dark:text-neutral-500">
            Market value
          </div>
        </div>

        {/* Cash Deposits */}
        <div className="p-4 rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 space-y-1">
          <div className="flex items-center justify-between text-xs font-medium text-neutral-500 dark:text-neutral-400 font-sans">
            <span className="uppercase font-mono text-[10px] tracking-wider">Portfolio Cash</span>
            <Coins className="w-3.5 h-3.5 text-neutral-400" />
          </div>
          <div className="text-2xl font-semibold font-sans tabular-nums tracking-tight text-neutral-900 dark:text-neutral-100">
            {formatMinor(data.portfolio_cash_minor, data.primary_currency_exponent)}
            <span className="text-xs font-mono font-normal ml-1.5 text-neutral-500 dark:text-neutral-400">
              {data.primary_currency}
            </span>
          </div>
          <div className="text-[10px] font-mono text-neutral-500 dark:text-neutral-500">
            Uninvested deposits
          </div>
        </div>
      </div>

      {/* Multi-currency breakdown pills */}
      {hasMultipleCurrencies && (
        <div className="flex flex-wrap items-center gap-2 pt-1 text-[11px] font-mono text-neutral-500 dark:text-neutral-400">
          <span className="text-[10px] uppercase tracking-wider text-neutral-400 dark:text-neutral-500">
            Currencies:
          </span>
          {data.native_breakdowns.map((b) => (
            <span
              key={b.currency}
              className="inline-flex items-center gap-1.5 px-2 py-0.5 rounded bg-neutral-100 dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800"
            >
              <span className="font-semibold text-neutral-700 dark:text-neutral-300">{b.currency}</span>
              <span>
                {formatMinor(
                  b.ledger_cash_minor + b.portfolio_market_value_minor + b.portfolio_cash_minor,
                  b.currency_exponent,
                )}
              </span>
              {b.currency !== data.primary_currency && (
                <span className="text-neutral-400 dark:text-neutral-500">
                  ≈ {formatMinor(b.converted_total_minor, data.primary_currency_exponent)} {data.primary_currency}
                </span>
              )}
            </span>
          ))}
        </div>
      )}
    </div>
  );
};
