import React from 'react';
import { PieChart, DollarSign, Coins, TrendingUp } from 'lucide-react';
import { Holding, CashBalance } from './types';

const KNOWN_CRYPTO = new Set([
  'BTC', 'ETH', 'SOL', 'DOGE', 'ADA', 'XRP', 'DOT', 'AVAX', 'MATIC', 'LINK',
  'LTC', 'BCH', 'XLM', 'UNI', 'ATOM', 'XMR', 'ETC', 'NEAR', 'ALGO', 'FIL',
]);

function isCrypto(symbol: string): boolean {
  const upper = symbol.toUpperCase().trim();
  return KNOWN_CRYPTO.has(upper) || upper.endsWith('-USD') || upper.endsWith('-USDT');
}

function formatAmount(minor: number, exponent: number): string {
  const full = (minor / 10 ** exponent).toFixed(exponent);
  return Number(full).toLocaleString(undefined, {
    minimumFractionDigits: Math.min(exponent, 2),
    maximumFractionDigits: Math.min(exponent, 2),
  });
}

interface AllocationCardProps {
  holdings: Holding[];
  cashBalances: CashBalance[];
}

export const AllocationCard: React.FC<AllocationCardProps> = ({ holdings, cashBalances }) => {
  // Determine primary currency (from holdings or cash or default USD)
  const primaryCurrency = holdings[0]?.currency || cashBalances[0]?.currency || 'USD';
  const currencyExponent = holdings[0]?.currency_exponent ?? cashBalances[0]?.currency_exponent ?? 2;

  // Compute values for holdings sharing primary currency
  let stocksMinor = 0;
  let cryptoMinor = 0;

  for (const h of holdings) {
    if (h.currency !== primaryCurrency) continue;
    if (h.current_price_minor) {
      // (quantity_minor * price_minor) / 10^8
      const val = Math.floor((h.quantity_minor * h.current_price_minor) / 10 ** 8);
      if (isCrypto(h.symbol)) {
        cryptoMinor += val;
      } else {
        stocksMinor += val;
      }
    }
  }

  const primaryCash = cashBalances.find((c) => c.currency === primaryCurrency);
  const cashMinor = primaryCash ? primaryCash.balance_minor : 0;

  const totalMinor = stocksMinor + cryptoMinor + cashMinor;

  if (totalMinor <= 0 && holdings.length === 0) {
    return null;
  }

  const stocksPct = totalMinor > 0 ? (stocksMinor / totalMinor) * 100 : 0;
  const cryptoPct = totalMinor > 0 ? (cryptoMinor / totalMinor) * 100 : 0;
  const cashPct = totalMinor > 0 ? (cashMinor / totalMinor) * 100 : 0;

  return (
    <div className="bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded-xl p-4 sm:p-5 shadow-xs space-y-4">
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-2">
        <div className="flex items-center gap-2">
          <PieChart className="w-5 h-5 text-blue-600 dark:text-blue-400" />
          <h2 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">Asset Allocation</h2>
          <span className="text-xs text-neutral-400">({primaryCurrency})</span>
        </div>
        <div className="text-xs text-neutral-500 dark:text-neutral-400">
          Total Net Worth:{' '}
          <span className="font-bold text-neutral-900 dark:text-neutral-100 text-sm">
            {formatAmount(totalMinor, currencyExponent)} {primaryCurrency}
          </span>
        </div>
      </div>

      {/* Segmented Distribution Bar */}
      <div className="h-3 w-full bg-neutral-100 dark:bg-neutral-800 rounded-full overflow-hidden flex gap-0.5">
        {stocksPct > 0 && (
          <div
            style={{ width: `${stocksPct}%` }}
            className="bg-blue-600 dark:bg-blue-500 h-full transition-all duration-500"
            title={`Equities & ETFs: ${stocksPct.toFixed(1)}%`}
          />
        )}
        {cryptoPct > 0 && (
          <div
            style={{ width: `${cryptoPct}%` }}
            className="bg-amber-500 dark:bg-amber-400 h-full transition-all duration-500"
            title={`Cryptocurrencies: ${cryptoPct.toFixed(1)}%`}
          />
        )}
        {cashPct > 0 && (
          <div
            style={{ width: `${cashPct}%` }}
            className="bg-emerald-600 dark:bg-emerald-500 h-full transition-all duration-500"
            title={`Cash Balance: ${cashPct.toFixed(1)}%`}
          />
        )}
      </div>

      {/* Allocation Metrics Cards */}
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-3 pt-1">
        {/* Stocks & ETFs */}
        <div className="p-3 bg-neutral-50 dark:bg-neutral-850 rounded-lg border border-neutral-200/70 dark:border-neutral-800/70 flex items-center justify-between">
          <div className="space-y-0.5">
            <div className="flex items-center gap-1.5 text-xs text-neutral-500 dark:text-neutral-400">
              <span className="w-2.5 h-2.5 rounded-full bg-blue-600 dark:bg-blue-500 inline-block" />
              <TrendingUp className="w-3.5 h-3.5 text-blue-600 dark:text-blue-400" />
              <span>Equities / ETFs</span>
            </div>
            <div className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
              {formatAmount(stocksMinor, currencyExponent)} {primaryCurrency}
            </div>
          </div>
          <div className="text-right">
            <span className="text-xs font-bold text-blue-600 dark:text-blue-400 font-mono">
              {stocksPct.toFixed(1)}%
            </span>
          </div>
        </div>

        {/* Crypto */}
        <div className="p-3 bg-neutral-50 dark:bg-neutral-850 rounded-lg border border-neutral-200/70 dark:border-neutral-800/70 flex items-center justify-between">
          <div className="space-y-0.5">
            <div className="flex items-center gap-1.5 text-xs text-neutral-500 dark:text-neutral-400">
              <span className="w-2.5 h-2.5 rounded-full bg-amber-500 dark:bg-amber-400 inline-block" />
              <Coins className="w-3.5 h-3.5 text-amber-500 dark:text-amber-400" />
              <span>Crypto</span>
            </div>
            <div className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
              {formatAmount(cryptoMinor, currencyExponent)} {primaryCurrency}
            </div>
          </div>
          <div className="text-right">
            <span className="text-xs font-bold text-amber-600 dark:text-amber-400 font-mono">
              {cryptoPct.toFixed(1)}%
            </span>
          </div>
        </div>

        {/* Cash */}
        <div className="p-3 bg-neutral-50 dark:bg-neutral-850 rounded-lg border border-neutral-200/70 dark:border-neutral-800/70 flex items-center justify-between">
          <div className="space-y-0.5">
            <div className="flex items-center gap-1.5 text-xs text-neutral-500 dark:text-neutral-400">
              <span className="w-2.5 h-2.5 rounded-full bg-emerald-600 dark:bg-emerald-500 inline-block" />
              <DollarSign className="w-3.5 h-3.5 text-emerald-600 dark:text-emerald-500" />
              <span>Cash Balance</span>
            </div>
            <div className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
              {formatAmount(cashMinor, currencyExponent)} {primaryCurrency}
            </div>
          </div>
          <div className="text-right">
            <span className="text-xs font-bold text-emerald-600 dark:text-emerald-400 font-mono">
              {cashPct.toFixed(1)}%
            </span>
          </div>
        </div>
      </div>
    </div>
  );
};
