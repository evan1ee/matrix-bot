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
  const primaryCurrency = holdings[0]?.currency || cashBalances[0]?.currency || 'USD';
  const currencyExponent = holdings[0]?.currency_exponent ?? cashBalances[0]?.currency_exponent ?? 2;

  let stocksMinor = 0;
  let cryptoMinor = 0;

  for (const h of holdings) {
    if (h.currency !== primaryCurrency) continue;
    if (h.current_price_minor) {
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

  const stocksPct = totalMinor > 0 ? (stocksMinor / totalMinor) * 100 : 0;
  const cryptoPct = totalMinor > 0 ? (cryptoMinor / totalMinor) * 100 : 0;
  const cashPct = totalMinor > 0 ? (cashMinor / totalMinor) * 100 : 0;

  // SVG Donut metrics
  const radius = 46;
  const circumference = 2 * Math.PI * radius; // ~289.027

  const stocksOffset = 0;
  const cryptoOffset = -((stocksPct / 100) * circumference);
  const cashOffset = -(((stocksPct + cryptoPct) / 100) * circumference);

  return (
    <div className="bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded-xl p-4 sm:p-5 shadow-xs flex flex-col justify-between h-full space-y-3">
      {/* Header matching PerformanceChart */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-2">
        <div>
          <div className="flex items-center gap-2">
            <h3 className="text-xs font-semibold uppercase tracking-wider text-neutral-500 font-mono">
              Asset Allocation ({primaryCurrency})
            </h3>
            <span className="flex items-center gap-1 text-[11px] text-neutral-400 font-mono bg-neutral-100 dark:bg-neutral-800 px-1.5 py-0.5 rounded">
              <PieChart className="w-3 h-3 text-neutral-500" />
              {holdings.length} {holdings.length === 1 ? 'Holding' : 'Holdings'}
            </span>
          </div>
          <div className="flex items-baseline gap-2 mt-1">
            <span className="text-xl sm:text-2xl font-bold text-neutral-900 dark:text-neutral-100 font-sans">
              {formatAmount(totalMinor, currencyExponent)} {primaryCurrency}
            </span>
            <span className="text-xs font-mono text-neutral-500 dark:text-neutral-400">
              Total Net Worth
            </span>
          </div>
        </div>

        {/* Visual Pill matching Timeframe Selector */}
        <div className="flex items-center gap-1 bg-neutral-100 dark:bg-neutral-800 px-2.5 py-1 rounded-lg text-xs font-mono text-neutral-600 dark:text-neutral-300 self-start sm:self-auto">
          <span>{totalMinor > 0 ? 'Holdings & Cash' : 'Empty'}</span>
        </div>
      </div>

      {/* Middle Donut Chart & Category Distribution */}
      <div className="relative w-full flex-1 flex flex-col sm:flex-row items-center justify-between gap-6 py-2">
        {/* SVG Donut */}
        <div className="relative w-36 h-36 shrink-0 flex items-center justify-center">
          <svg viewBox="0 0 120 120" className="w-full h-full -rotate-90">
            <circle
              cx="60"
              cy="60"
              r={radius}
              fill="none"
              stroke="currentColor"
              className="text-neutral-100 dark:text-neutral-800"
              strokeWidth="12"
            />
            {totalMinor > 0 && (
              <>
                {stocksPct > 0 && (
                  <circle
                    cx="60"
                    cy="60"
                    r={radius}
                    fill="none"
                    stroke="#3b82f6"
                    strokeWidth="12"
                    strokeDasharray={`${(stocksPct / 100) * circumference} ${circumference}`}
                    strokeDashoffset={stocksOffset}
                    className="transition-all duration-500"
                  />
                )}
                {cryptoPct > 0 && (
                  <circle
                    cx="60"
                    cy="60"
                    r={radius}
                    fill="none"
                    stroke="#f59e0b"
                    strokeWidth="12"
                    strokeDasharray={`${(cryptoPct / 100) * circumference} ${circumference}`}
                    strokeDashoffset={cryptoOffset}
                    className="transition-all duration-500"
                  />
                )}
                {cashPct > 0 && (
                  <circle
                    cx="60"
                    cy="60"
                    r={radius}
                    fill="none"
                    stroke="#10b981"
                    strokeWidth="12"
                    strokeDasharray={`${(cashPct / 100) * circumference} ${circumference}`}
                    strokeDashoffset={cashOffset}
                    className="transition-all duration-500"
                  />
                )}
              </>
            )}
          </svg>
          <div className="absolute inset-0 flex flex-col items-center justify-center pointer-events-none">
            <span className="text-sm font-bold font-mono text-neutral-900 dark:text-neutral-100">
              {totalMinor > 0 ? `${(100).toFixed(0)}%` : '0%'}
            </span>
            <span className="text-[10px] font-mono text-neutral-400 uppercase">
              Allocated
            </span>
          </div>
        </div>

        {/* Category Progress Bars */}
        <div className="flex-1 w-full space-y-3">
          {/* Equities */}
          <div className="space-y-1">
            <div className="flex items-center justify-between text-xs">
              <div className="flex items-center gap-1.5 text-neutral-700 dark:text-neutral-300 font-medium">
                <span className="w-2 h-2 rounded-full bg-blue-500 shrink-0" />
                <TrendingUp className="w-3.5 h-3.5 text-blue-500" />
                <span>Equities / ETFs</span>
              </div>
              <span className="font-mono font-semibold text-blue-600 dark:text-blue-400">
                {stocksPct.toFixed(1)}%
              </span>
            </div>
            <div className="h-1.5 w-full bg-neutral-100 dark:bg-neutral-800 rounded-full overflow-hidden">
              <div
                style={{ width: `${stocksPct}%` }}
                className="h-full bg-blue-500 rounded-full transition-all duration-500"
              />
            </div>
          </div>

          {/* Crypto */}
          <div className="space-y-1">
            <div className="flex items-center justify-between text-xs">
              <div className="flex items-center gap-1.5 text-neutral-700 dark:text-neutral-300 font-medium">
                <span className="w-2 h-2 rounded-full bg-amber-500 shrink-0" />
                <Coins className="w-3.5 h-3.5 text-amber-500" />
                <span>Cryptocurrencies</span>
              </div>
              <span className="font-mono font-semibold text-amber-600 dark:text-amber-400">
                {cryptoPct.toFixed(1)}%
              </span>
            </div>
            <div className="h-1.5 w-full bg-neutral-100 dark:bg-neutral-800 rounded-full overflow-hidden">
              <div
                style={{ width: `${cryptoPct}%` }}
                className="h-full bg-amber-500 rounded-full transition-all duration-500"
              />
            </div>
          </div>

          {/* Cash */}
          <div className="space-y-1">
            <div className="flex items-center justify-between text-xs">
              <div className="flex items-center gap-1.5 text-neutral-700 dark:text-neutral-300 font-medium">
                <span className="w-2 h-2 rounded-full bg-emerald-500 shrink-0" />
                <DollarSign className="w-3.5 h-3.5 text-emerald-500" />
                <span>Cash Balance</span>
              </div>
              <span className="font-mono font-semibold text-emerald-600 dark:text-emerald-400">
                {cashPct.toFixed(1)}%
              </span>
            </div>
            <div className="h-1.5 w-full bg-neutral-100 dark:bg-neutral-800 rounded-full overflow-hidden">
              <div
                style={{ width: `${cashPct}%` }}
                className="h-full bg-emerald-500 rounded-full transition-all duration-500"
              />
            </div>
          </div>
        </div>
      </div>

      {/* Footer 3-column breakdown matching PerformanceChart */}
      <div className="grid grid-cols-2 sm:grid-cols-3 gap-2 pt-2 border-t border-neutral-100 dark:border-neutral-800/80 text-xs">
        <div>
          <span className="text-[10px] uppercase font-mono text-neutral-400 block">Equities & ETFs</span>
          <span className="font-semibold text-neutral-800 dark:text-neutral-200 font-mono">
            {formatAmount(stocksMinor, currencyExponent)} {primaryCurrency}
          </span>
        </div>
        <div>
          <span className="text-[10px] uppercase font-mono text-neutral-400 block">Crypto Value</span>
          <span className="font-semibold text-neutral-800 dark:text-neutral-200 font-mono">
            {formatAmount(cryptoMinor, currencyExponent)} {primaryCurrency}
          </span>
        </div>
        <div className="col-span-2 sm:col-span-1">
          <span className="text-[10px] uppercase font-mono text-neutral-400 block">Cash Reserve</span>
          <span className="font-bold text-neutral-900 dark:text-neutral-100 font-mono">
            {formatAmount(cashMinor, currencyExponent)} {primaryCurrency}
          </span>
        </div>
      </div>
    </div>
  );
};
