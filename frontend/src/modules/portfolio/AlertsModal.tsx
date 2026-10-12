import React, { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Bell, BellRing, Trash2, X, Plus, AlertCircle, TrendingUp, TrendingDown, Check } from 'lucide-react';
import { fetchApi } from '../../core/apiClient';
import { formatDateInZone, useSettings } from '../../core/settings';
import { PriceAlert, AlertCondition } from './types';
import { TickerSearchInput } from './TickerSearchInput';

interface AlertsModalProps {
  initialSymbol?: string;
  defaultCurrency?: string;
  onClose: () => void;
}

function formatAmount(minor: number, exponent: number): string {
  const full = (minor / 10 ** exponent).toFixed(exponent);
  return Number(full).toLocaleString(undefined, {
    minimumFractionDigits: exponent,
    maximumFractionDigits: exponent,
  });
}

export const AlertsModal: React.FC<AlertsModalProps> = ({
  initialSymbol = '',
  defaultCurrency = 'USD',
  onClose,
}) => {
  const queryClient = useQueryClient();
  const { timezone } = useSettings();

  const [symbol, setSymbol] = useState(initialSymbol);
  const [targetPrice, setTargetPrice] = useState('');
  const [currency, setCurrency] = useState(defaultCurrency);
  const [condition, setCondition] = useState<AlertCondition>('above');
  const [error, setError] = useState<string | null>(null);

  const { data: alerts = [], isLoading } = useQuery({
    queryKey: ['portfolio-alerts'],
    queryFn: () => fetchApi<PriceAlert[]>('/portfolio/alerts'),
  });

  const invalidate = () => {
    queryClient.invalidateQueries({ queryKey: ['portfolio-alerts'] });
  };

  const createMutation = useMutation({
    mutationFn: (body: {
      symbol: string;
      target_price: string;
      currency: string;
      condition: AlertCondition;
    }) =>
      fetchApi<PriceAlert>('/portfolio/alerts', {
        method: 'POST',
        body: JSON.stringify(body),
      }),
    onSuccess: () => {
      invalidate();
      setTargetPrice('');
      setError(null);
    },
    onError: (err: Error) => {
      setError(err.message || 'Failed to create alert.');
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: number) =>
      fetchApi(`/portfolio/alerts/${id}`, {
        method: 'DELETE',
      }),
    onSuccess: invalidate,
  });

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!symbol.trim() || !targetPrice.trim()) {
      setError('Please provide both a symbol and a target price.');
      return;
    }
    setError(null);
    createMutation.mutate({
      symbol: symbol.trim().toUpperCase(),
      target_price: targetPrice.trim(),
      currency: currency.trim().toUpperCase(),
      condition,
    });
  };

  return (
    <div className="fixed inset-0 z-50 bg-black/50 flex items-center justify-center p-4">
      <div className="bg-white dark:bg-neutral-900 rounded-xl border border-neutral-200 dark:border-neutral-800 p-5 max-w-lg w-full space-y-5 shadow-2xl max-h-[90vh] flex flex-col">
        {/* Header */}
        <div className="flex items-center justify-between pb-3 border-b border-neutral-100 dark:border-neutral-800">
          <div className="flex items-center gap-2">
            <div className="p-1.5 rounded-lg bg-neutral-100 dark:bg-neutral-800 text-neutral-700 dark:text-neutral-300">
              <BellRing className="w-5 h-5" />
            </div>
            <div>
              <h3 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
                Market Price & Volatility Alerts
              </h3>
              <p className="text-xs text-neutral-500 mt-0.5">
                Delivered directly to your Matrix chat when targets are reached.
              </p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="p-1.5 rounded text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition cursor-pointer"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Create Alert Form */}
        <form onSubmit={handleSubmit} className="p-3.5 bg-neutral-50 dark:bg-neutral-800/50 rounded-lg space-y-3 border border-neutral-200/60 dark:border-neutral-800">
          <span className="text-xs font-semibold text-neutral-800 dark:text-neutral-200 block">
            Create New Price Alert
          </span>

          <div className="grid grid-cols-1 sm:grid-cols-12 gap-2.5 items-end">
            <div className="sm:col-span-4 space-y-1">
              <label className="text-[10px] uppercase font-mono text-neutral-500">Symbol</label>
              <TickerSearchInput
                value={symbol}
                onChange={setSymbol}
                placeholder="BTC-USD, AAPL"
                className="w-full h-9 px-2.5 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-700 text-xs font-mono uppercase text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500"
              />
            </div>

            <div className="sm:col-span-3 space-y-1">
              <label className="text-[10px] uppercase font-mono text-neutral-500">Condition</label>
              <div className="flex h-9 rounded-md border border-neutral-300 dark:border-neutral-700 overflow-hidden bg-white dark:bg-neutral-900 p-0.5">
                <button
                  type="button"
                  onClick={() => setCondition('above')}
                  className={`flex-1 flex items-center justify-center gap-1 text-[11px] font-mono rounded transition cursor-pointer ${
                    condition === 'above'
                      ? 'bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900 font-semibold shadow-xs'
                      : 'text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-100'
                  }`}
                >
                  <TrendingUp className="w-3 h-3" /> ≥
                </button>
                <button
                  type="button"
                  onClick={() => setCondition('below')}
                  className={`flex-1 flex items-center justify-center gap-1 text-[11px] font-mono rounded transition cursor-pointer ${
                    condition === 'below'
                      ? 'bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900 font-semibold shadow-xs'
                      : 'text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-100'
                  }`}
                >
                  <TrendingDown className="w-3 h-3" /> ≤
                </button>
              </div>
            </div>

            <div className="sm:col-span-3 space-y-1">
              <label className="text-[10px] uppercase font-mono text-neutral-500">Target Price</label>
              <input
                type="text"
                inputMode="decimal"
                value={targetPrice}
                onChange={(e) => setTargetPrice(e.target.value)}
                placeholder="e.g. 70000"
                className="w-full h-9 px-2.5 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-700 text-xs font-mono text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500"
              />
            </div>

            <div className="sm:col-span-2">
              <button
                type="submit"
                disabled={createMutation.isPending || !symbol.trim() || !targetPrice.trim()}
                className="w-full h-9 flex items-center justify-center gap-1 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-semibold hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50 cursor-pointer"
              >
                <Plus className="w-3.5 h-3.5" />
                Add
              </button>
            </div>
          </div>

          {error && (
            <div className="flex items-center gap-1.5 text-xs text-rose-600 dark:text-rose-400 font-mono">
              <AlertCircle className="w-3.5 h-3.5 shrink-0" />
              <span>{error}</span>
            </div>
          )}
        </form>

        {/* Existing Alerts List */}
        <div className="flex-1 overflow-y-auto space-y-2 pr-1">
          <div className="flex items-center justify-between text-xs font-semibold text-neutral-500 uppercase tracking-wider font-mono">
            <span>Your Active & Past Alerts ({alerts.length})</span>
          </div>

          {isLoading ? (
            <p className="text-xs text-neutral-400 font-mono py-4 text-center">Loading alerts…</p>
          ) : alerts.length === 0 ? (
            <div className="text-center py-6 text-neutral-400 text-xs space-y-1">
              <Bell className="w-7 h-7 mx-auto stroke-1 text-neutral-300 dark:text-neutral-700" />
              <p>No price alerts set yet.</p>
              <p className="text-[11px] text-neutral-500">
                Set a target price above to get notified on Matrix.
              </p>
            </div>
          ) : (
            <div className="space-y-1.5">
              {alerts.map((a: PriceAlert) => (
                <div
                  key={a.id}
                  className={`flex items-center justify-between p-2.5 rounded-lg border transition ${
                    a.is_triggered
                      ? 'bg-neutral-50/50 dark:bg-neutral-900/40 border-neutral-200/60 dark:border-neutral-800/60 opacity-60'
                      : 'bg-white dark:bg-neutral-800/40 border-neutral-200 dark:border-neutral-800'
                  }`}
                >
                  <div className="flex items-center gap-2.5">
                    <span
                      className={`p-1 rounded text-xs ${
                        a.condition === 'above'
                          ? 'bg-emerald-100 dark:bg-emerald-950/60 text-emerald-700 dark:text-emerald-300'
                          : 'bg-rose-100 dark:bg-rose-950/60 text-rose-700 dark:text-rose-300'
                      }`}
                    >
                      {a.condition === 'above' ? <TrendingUp className="w-3.5 h-3.5" /> : <TrendingDown className="w-3.5 h-3.5" />}
                    </span>
                    <div>
                      <div className="flex items-center gap-2">
                        <span className="font-bold text-xs font-mono text-neutral-900 dark:text-neutral-100">
                          {a.symbol}
                        </span>
                        <span className="text-xs font-mono text-neutral-600 dark:text-neutral-400">
                          {a.condition === 'above' ? '≥' : '≤'}{' '}
                          {formatAmount(a.target_price_minor, a.currency_exponent)} {a.currency}
                        </span>
                      </div>
                      <span className="text-[10px] text-neutral-400 font-mono">
                        {a.is_triggered
                          ? `Triggered on ${formatDateInZone(a.triggered_at || a.created_at, timezone)}`
                          : `Created on ${formatDateInZone(a.created_at, timezone)}`}
                      </span>
                    </div>
                  </div>

                  <div className="flex items-center gap-2">
                    {a.is_triggered ? (
                      <span className="px-2 py-0.5 rounded text-[10px] font-mono bg-neutral-100 dark:bg-neutral-800 text-neutral-500">
                        Triggered
                      </span>
                    ) : (
                      <span className="px-2 py-0.5 rounded text-[10px] font-mono font-medium bg-neutral-100 dark:bg-neutral-800 text-neutral-700 dark:text-neutral-300 border border-neutral-200 dark:border-neutral-700">
                        Active
                      </span>
                    )}

                    <button
                      onClick={() => deleteMutation.mutate(a.id)}
                      disabled={deleteMutation.isPending}
                      className="p-1 rounded text-neutral-400 hover:text-rose-600 dark:hover:text-rose-400 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition cursor-pointer"
                      title="Delete alert"
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
};
