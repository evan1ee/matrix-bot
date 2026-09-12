import React, { useEffect, useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import { FrontendModule } from '../../core/types';
import {
  TrendingUp,
  Plus,
  Trash2,
  AlertCircle,
  Tag,
  X,
  Check,
  Pencil,
  ArrowDownRight,
  ArrowUpRight,
  Wallet,
  RefreshCw,
  BookmarkPlus,
  AlertTriangle,
  Bell,
  Briefcase,
  Sparkles,
} from 'lucide-react';
import { triggerAiPrompt } from '../ai';
import { CustomSelect } from '../../components/CustomSelect';
import { DateTimePicker } from '../../components/DateTimePicker';
import { CURRENCY_OPTIONS, useSettings, zonedInputToUtcIso, nowInZone } from '../../core/settings';
import { AllocationCard } from './AllocationCard';
import { PerformanceChart } from './PerformanceChart';
import { AlertsModal } from './AlertsModal';
import { ActivitySection } from './ActivitySection';
import { TickerSearchInput } from './TickerSearchInput';
import {
  Holding,
  PortfolioCurrencySummary,
  SellResponse,
  CashBalance,
  RefreshSummary,
} from './types';

export * from './types';

/** Decimal places quantity is stored at — mirrors the backend's `QUANTITY_EXPONENT`. */
const QUANTITY_EXPONENT = 8;

/**
 * Renders minor units as a decimal string. Integer maths only, same reason
 * `finance`'s `formatMinor` avoids a float: dividing into one on the way to
 * display is exactly the rounding this whole design avoids on the server.
 */
function formatMinor(minor: number, exponent: number): string {
  const negative = minor < 0;
  const digits = Math.abs(minor).toString().padStart(exponent + 1, '0');
  const text =
    exponent === 0
      ? digits
      : `${digits.slice(0, digits.length - exponent)}.${digits.slice(digits.length - exponent)}`;
  return negative ? `-${text}` : text;
}

/** Like `formatMinor`, but trims trailing zeros — `10.00000000` shares reads as
 *  noise; a plain `10` or `1.5` is what the user actually typed. */
function formatQuantity(minor: number): string {
  const full = formatMinor(minor, QUANTITY_EXPONENT);
  const [whole, frac] = full.split('.');
  if (!frac) return full;
  const trimmed = frac.replace(/0+$/, '');
  return trimmed ? `${whole}.${trimmed}` : whole;
}

function formatSigned(minor: number, exponent: number): string {
  return minor < 0 ? formatMinor(minor, exponent) : `+${formatMinor(minor, exponent)}`;
}

function plColor(minor: number): string {
  return minor < 0 ? 'text-rose-600 dark:text-rose-400' : 'text-emerald-600 dark:text-emerald-400';
}

/** Same "unlisted currency stays selectable" reasoning as `finance`'s
 *  `currencyOptionsFor` — a Matrix `!portfolio buy` can write a currency this
 *  list does not offer. */
function currencyOptionsFor(current: string) {
  if (!current || CURRENCY_OPTIONS.some((o) => o.value === current)) {
    return CURRENCY_OPTIONS;
  }
  return [...CURRENCY_OPTIONS, { value: current, label: current }];
}

interface SellRowProps {
  holding: Holding;
  saving: boolean;
  onCancel: () => void;
  onSell: (quantity: string, price: string, executedAt?: string) => Promise<void>;
  timezone: string;
}

const SellRow: React.FC<SellRowProps> = ({ holding, saving, onCancel, onSell, timezone }) => {
  const [quantity, setQuantity] = useState('');
  const [price, setPrice] = useState('');
  const [sellDate, setSellDate] = useState('');
  const [error, setError] = useState<string | null>(null);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!quantity.trim() || !price.trim()) {
      setError('Enter both a quantity and a price.');
      return;
    }
    setError(null);
    try {
      const executedAt = sellDate ? zonedInputToUtcIso(sellDate, timezone) ?? undefined : undefined;
      await onSell(quantity.trim(), price.trim(), executedAt);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to record the sale.');
    }
  };

  return (
    <form
      onSubmit={submit}
      data-testid="sell-row"
      className="py-3 px-2 space-y-3 bg-neutral-100/60 dark:bg-neutral-900/30 rounded"
    >
      <div className="grid grid-cols-1 sm:grid-cols-4 gap-3">
        <div className="flex flex-col gap-1">
          <div className="flex items-center justify-between">
            <span className="text-[10px] uppercase font-mono text-neutral-500">
              Sell quantity (of {formatQuantity(holding.quantity_minor)})
            </span>
            <button
              type="button"
              onClick={() => setQuantity(formatQuantity(holding.quantity_minor))}
              className="text-[10px] font-mono text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-200 underline decoration-dotted cursor-pointer"
              title="Click to fill max available quantity"
            >
              MAX
            </button>
          </div>
          <div className="relative">
            <input
              type="text"
              inputMode="decimal"
              value={quantity}
              onChange={(e) => setQuantity(e.target.value)}
              placeholder="e.g. 5"
              className="w-full pl-3 pr-14 py-2 rounded bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-mono"
            />
            <button
              type="button"
              onClick={() => setQuantity(formatQuantity(holding.quantity_minor))}
              className="absolute right-1.5 top-1/2 -translate-y-1/2 px-2 py-0.5 rounded text-[10px] font-mono font-bold bg-neutral-200/80 dark:bg-neutral-800 text-neutral-700 dark:text-neutral-300 hover:bg-neutral-300 dark:hover:bg-neutral-700 hover:text-neutral-900 dark:hover:text-neutral-100 transition cursor-pointer"
              title={`Fill max quantity: ${formatQuantity(holding.quantity_minor)}`}
            >
              MAX
            </button>
          </div>
        </div>
        <label className="flex flex-col gap-1">
          <span className="text-[10px] uppercase font-mono text-neutral-500">
            Sale price ({holding.currency})
          </span>
          <input
            type="text"
            inputMode="decimal"
            value={price}
            onChange={(e) => setPrice(e.target.value)}
            placeholder="e.g. 180.00"
            className="px-3 py-2 rounded bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-mono"
          />
        </label>
        <label className="flex flex-col gap-1">
          <span className="text-[10px] uppercase font-mono text-neutral-500">
            Sale Date / Time (Optional)
          </span>
          <DateTimePicker
            value={sellDate}
            onChange={setSellDate}
            placeholder="Now (current time)"
            now={nowInZone(timezone)}
          />
        </label>
        <div className="flex items-end gap-2">
          <button
            type="submit"
            disabled={saving}
            className="inline-flex items-center gap-2 px-3 py-2 rounded bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50"
          >
            <ArrowDownRight className="w-3.5 h-3.5" />
            {saving ? 'Selling…' : 'Sell'}
          </button>
          <button
            type="button"
            onClick={onCancel}
            className="inline-flex items-center gap-2 px-3 py-2 rounded border border-neutral-300 dark:border-neutral-700 text-neutral-600 dark:text-neutral-300 text-xs font-medium hover:bg-neutral-200 dark:hover:bg-neutral-800 transition"
          >
            <X className="w-3.5 h-3.5" />
            Cancel
          </button>
        </div>
      </div>
      {error && (
        <div className="flex items-center gap-2 p-2 rounded bg-red-100 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 text-red-800 dark:text-red-300 text-xs font-mono">
          <AlertCircle className="w-4 h-4 shrink-0" />
          <span>{error}</span>
        </div>
      )}
    </form>
  );
};

interface NoteRowProps {
  holding: Holding;
  saving: boolean;
  onCancel: () => void;
  onSave: (note: string) => Promise<void>;
}

/// An empty note is a real value here — it clears the note — so unlike
/// `PriceRow` this form does not reject a blank field.
const NoteRow: React.FC<NoteRowProps> = ({ holding, saving, onCancel, onSave }) => {
  const [note, setNote] = useState(holding.note ?? '');
  const [error, setError] = useState<string | null>(null);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    try {
      await onSave(note.trim());
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save the note.');
    }
  };

  return (
    <form onSubmit={submit} className="flex items-center gap-2">
      <input
        type="text"
        autoFocus
        value={note}
        onChange={(e) => setNote(e.target.value)}
        placeholder="Note (leave blank to clear)"
        className="w-56 px-2 py-1 rounded bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-sans"
      />
      <button
        type="submit"
        disabled={saving}
        title="Save note"
        className="p-1.5 rounded text-neutral-600 dark:text-neutral-300 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-200 dark:hover:bg-neutral-900 transition"
      >
        <Check className="w-3.5 h-3.5" />
      </button>
      <button
        type="button"
        onClick={onCancel}
        title="Cancel"
        className="p-1.5 rounded text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-200 dark:hover:bg-neutral-900 transition"
      >
        <X className="w-3.5 h-3.5" />
      </button>
      {error && <span className="text-[11px] font-mono text-rose-600 dark:text-rose-400">{error}</span>}
    </form>
  );
};

interface PriceRowProps {
  holding: Holding;
  saving: boolean;
  onCancel: () => void;
  onSave: (price: string) => Promise<void>;
}

const PriceRow: React.FC<PriceRowProps> = ({ holding, saving, onCancel, onSave }) => {
  const [price, setPrice] = useState(
    holding.current_price_minor != null
      ? formatMinor(holding.current_price_minor, holding.currency_exponent)
      : ''
  );
  const [error, setError] = useState<string | null>(null);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!price.trim()) {
      setError('Enter a price.');
      return;
    }
    setError(null);
    try {
      await onSave(price.trim());
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save the price.');
    }
  };

  return (
    <form onSubmit={submit} className="flex items-center gap-2">
      <input
        type="text"
        inputMode="decimal"
        autoFocus
        value={price}
        onChange={(e) => setPrice(e.target.value)}
        placeholder={`Price (${holding.currency})`}
        className="w-28 px-2 py-1 rounded bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-xs text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-mono"
      />
      <button
        type="submit"
        disabled={saving}
        title="Save price"
        className="p-1.5 rounded text-neutral-600 dark:text-neutral-300 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-200 dark:hover:bg-neutral-900 transition"
      >
        <Check className="w-3.5 h-3.5" />
      </button>
      <button
        type="button"
        onClick={onCancel}
        title="Cancel"
        className="p-1.5 rounded text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-200 dark:hover:bg-neutral-900 transition"
      >
        <X className="w-3.5 h-3.5" />
      </button>
      {error && <span className="text-[11px] font-mono text-rose-600 dark:text-rose-400">{error}</span>}
    </form>
  );
};

export const PortfolioMainView: React.FC = () => {
  const queryClient = useQueryClient();
  const { currency: defaultCurrency, timezone } = useSettings();

  const [symbol, setSymbol] = useState('');
  const [quantity, setQuantity] = useState('');
  const [price, setPrice] = useState('');
  const [currency, setCurrency] = useState(defaultCurrency);
  const [note, setNote] = useState('');
  const [transactionDate, setTransactionDate] = useState('');
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [sellingId, setSellingId] = useState<number | null>(null);
  const [editingPriceId, setEditingPriceId] = useState<number | null>(null);
  const [editingNoteId, setEditingNoteId] = useState<number | null>(null);

  const [cashAmount, setCashAmount] = useState('');
  const [cashCurrency, setCashCurrency] = useState(defaultCurrency);
  const [cashDate, setCashDate] = useState('');
  const [cashError, setCashError] = useState<string | null>(null);

  // The first render happens before the settings query resolves, and the
  // preference can change while this view is mounted; either way the form
  // should show what the user actually chose.
  useEffect(() => {
    setCurrency(defaultCurrency);
    setCashCurrency(defaultCurrency);
  }, [defaultCurrency]);

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

  // A buy/sell also moves cash, so every mutation below invalidates all three
  // — the alternative is each one guessing which of the other two it touched.
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

  const [editingSymbolHolding, setEditingSymbolHolding] = useState<Holding | null>(null);
  const [newSymbolInput, setNewSymbolInput] = useState('');
  const [symbolError, setSymbolError] = useState<string | null>(null);
  const [refreshingHoldingId, setRefreshingHoldingId] = useState<number | null>(null);
  const [removingHolding, setRemovingHolding] = useState<Holding | null>(null);
  const [alertsModalOpen, setAlertsModalOpen] = useState(false);
  const [alertSymbol, setAlertSymbol] = useState('');
  const [alertCurrency, setAlertCurrency] = useState('USD');

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
        if (editingSymbolHolding) next.delete(editingSymbolHolding.symbol);
        return next;
      });
      setEditingSymbolHolding(null);
      setNewSymbolInput('');
      setSymbolError(null);
      invalidate();
    },
    onError: (err: Error) => setSymbolError(err.message),
  });

  const buyMutation = useMutation({
    mutationFn: (body: Record<string, unknown>) =>
      fetchApi<Holding>('/portfolio/holdings', { method: 'POST', body: JSON.stringify(body) }),
    onSuccess: () => {
      setSymbol('');
      setQuantity('');
      setPrice('');
      setNote('');
      setTransactionDate('');
      setErrorMsg(null);
      invalidate();
    },
    onError: (err: Error) => setErrorMsg(err.message),
  });

  const recordMutation = useMutation({
    mutationFn: (body: Record<string, unknown>) =>
      fetchApi<Holding>('/portfolio/holdings/record', { method: 'POST', body: JSON.stringify(body) }),
    onSuccess: () => {
      setSymbol('');
      setQuantity('');
      setPrice('');
      setNote('');
      setTransactionDate('');
      setErrorMsg(null);
      invalidate();
    },
    onError: (err: Error) => setErrorMsg(err.message),
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

  // `note: null` clears it, which is why the blank field is sent as null
  // rather than an empty string.
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
      setCashAmount('');
      setCashDate('');
      setCashError(null);
      invalidate();
    },
    onError: (err: Error) => setCashError(err.message),
  });

  const withdrawMutation = useMutation({
    mutationFn: (body: { amount: string; currency: string; executed_at?: string }) =>
      fetchApi<CashBalance>('/portfolio/cash/withdraw', { method: 'POST', body: JSON.stringify(body) }),
    onSuccess: () => {
      setCashAmount('');
      setCashDate('');
      setCashError(null);
      invalidate();
    },
    onError: (err: Error) => setCashError(err.message),
  });

  const submitCash = (action: 'deposit' | 'withdraw') => {
    if (!cashAmount.trim()) {
      setCashError('Enter an amount.');
      return;
    }
    const executed_at = cashDate ? zonedInputToUtcIso(cashDate, timezone) ?? undefined : undefined;
    const body = {
      amount: cashAmount.trim(),
      currency: cashCurrency.trim().toUpperCase(),
      executed_at,
    };
    if (action === 'deposit') {
      depositMutation.mutate(body);
    } else {
      withdrawMutation.mutate(body);
    }
  };

  const handleBuy = (e: React.FormEvent) => {
    e.preventDefault();
    if (!symbol.trim() || !quantity.trim() || !price.trim()) {
      setErrorMsg('Enter a symbol, quantity, and price.');
      return;
    }
    const executed_at = transactionDate ? zonedInputToUtcIso(transactionDate, timezone) ?? undefined : undefined;
    buyMutation.mutate({
      symbol: symbol.trim().toUpperCase(),
      // Sent as strings: a JSON number would be a double by the time it
      // reaches the server, and neither a fractional quantity nor a price
      // survives that exactly.
      quantity: quantity.trim(),
      price: price.trim(),
      currency: currency.trim().toUpperCase(),
      note: note.trim() || null,
      executed_at,
    });
  };

  const handleRecord = (e: React.MouseEvent) => {
    e.preventDefault();
    if (!symbol.trim() || !quantity.trim() || !price.trim()) {
      setErrorMsg('Enter a symbol, quantity, and cost price.');
      return;
    }
    const executed_at = transactionDate ? zonedInputToUtcIso(transactionDate, timezone) ?? undefined : undefined;
    recordMutation.mutate({
      symbol: symbol.trim().toUpperCase(),
      quantity: quantity.trim(),
      price: price.trim(),
      currency: currency.trim().toUpperCase(),
      note: note.trim() || null,
      executed_at,
    });
  };

  return (
    <div className="space-y-6">
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
      <div
        data-testid="cash-panel"
        className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4"
      >
        <div className="flex items-center gap-2 border-b border-neutral-200 dark:border-neutral-800 pb-3">
          <Wallet className="w-4 h-4 text-neutral-600 dark:text-neutral-300" />
          <h3 className="text-sm font-medium text-neutral-900 dark:text-neutral-200">Cash</h3>
        </div>

        {cashError && (
          <div className="flex items-center gap-2 p-3 rounded bg-red-100 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 text-red-800 dark:text-red-300 text-xs font-mono">
            <AlertCircle className="w-4 h-4 shrink-0" />
            <span>{cashError}</span>
          </div>
        )}

        {(cashBalances ?? []).length > 0 && (
          <div className="flex flex-wrap gap-2" data-testid="cash-balances">
            {(cashBalances ?? []).map((c) => (
              <span
                key={c.currency}
                className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-sm font-mono text-neutral-900 dark:text-neutral-100"
              >
                {formatMinor(c.balance_minor, c.currency_exponent)} {c.currency}
              </span>
            ))}
          </div>
        )}

        <div className="grid grid-cols-1 sm:grid-cols-12 gap-3 items-end">
          <label className="flex flex-col gap-1 sm:col-span-4">
            <span className="text-[10px] uppercase font-mono text-neutral-500">Amount</span>
            <input
              type="text"
              inputMode="decimal"
              placeholder="500.00"
              value={cashAmount}
              onChange={(e) => setCashAmount(e.target.value)}
              className="h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-mono"
            />
          </label>
          <label className="flex flex-col gap-1 sm:col-span-2">
            <span className="text-[10px] uppercase font-mono text-neutral-500">Currency</span>
            <CustomSelect
              value={cashCurrency}
              onChange={setCashCurrency}
              mono
              options={currencyOptionsFor(cashCurrency)}
            />
          </label>
          <label className="flex flex-col gap-1 sm:col-span-3">
            <span className="text-[10px] uppercase font-mono text-neutral-500">Date / Time (Optional)</span>
            <DateTimePicker
              value={cashDate}
              onChange={setCashDate}
              placeholder="Now (current time)"
              now={nowInZone(timezone)}
            />
          </label>
          <div className="flex gap-2 sm:col-span-3">
            <button
              type="button"
              onClick={() => submitCash('deposit')}
              disabled={depositMutation.isPending}
              className="h-9 flex-1 inline-flex items-center justify-center gap-1.5 px-3 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50 cursor-pointer"
            >
              <ArrowUpRight className="w-3.5 h-3.5" />
              Deposit
            </button>
            <button
              type="button"
              onClick={() => submitCash('withdraw')}
              disabled={withdrawMutation.isPending}
              className="h-9 flex-1 inline-flex items-center justify-center gap-1.5 px-3 rounded-md border border-neutral-300 dark:border-neutral-700 text-neutral-700 dark:text-neutral-300 text-xs font-medium hover:bg-neutral-200 dark:hover:bg-neutral-800 transition disabled:opacity-50 cursor-pointer"
            >
              <ArrowDownRight className="w-3.5 h-3.5" />
              Withdraw
            </button>
          </div>
        </div>
        <p className="text-[11px] font-mono text-neutral-500">
          Buying a position debits cash automatically; selling credits it back.
        </p>
      </div>

      {/* Add Holding */}
      <div
        data-testid="buy-panel"
        className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4"
      >
        <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3">
          <div className="flex items-center gap-2">
            <Briefcase className="w-4 h-4 text-neutral-600 dark:text-neutral-300" />
            <h3 className="text-sm font-medium text-neutral-900 dark:text-neutral-200">
              Add Holding
            </h3>
          </div>
          <span className="text-xs font-mono text-neutral-500">Cash / Manual Record</span>
        </div>

        {errorMsg && (
          <div className="flex items-center gap-2 p-3 rounded bg-red-100 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 text-red-800 dark:text-red-300 text-xs font-mono">
            <AlertCircle className="w-4 h-4 shrink-0" />
            <span>{errorMsg}</span>
          </div>
        )}

        <form onSubmit={handleBuy} className="space-y-3">
          <div className="grid grid-cols-1 sm:grid-cols-12 gap-3">
            <div className="flex flex-col gap-1 sm:col-span-3">
              <span className="text-[10px] uppercase font-mono text-neutral-500">Symbol</span>
              <TickerSearchInput
                value={symbol}
                onChange={setSymbol}
                placeholder="e.g. AAPL, BTC, VOO"
                className="h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-mono uppercase"
              />
            </div>

            <label className="flex flex-col gap-1 sm:col-span-2">
              <span className="text-[10px] uppercase font-mono text-neutral-500">Quantity</span>
              <input
                type="text"
                inputMode="decimal"
                placeholder="10"
                value={quantity}
                onChange={(e) => setQuantity(e.target.value)}
                className="h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-mono"
              />
            </label>

            <label className="flex flex-col gap-1 sm:col-span-2">
              <span className="text-[10px] uppercase font-mono text-neutral-500">Price</span>
              <input
                type="text"
                inputMode="decimal"
                placeholder="150.00"
                value={price}
                onChange={(e) => setPrice(e.target.value)}
                className="h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-mono"
              />
            </label>

            <label className="flex flex-col gap-1 sm:col-span-2">
              <span className="text-[10px] uppercase font-mono text-neutral-500">Currency</span>
              <CustomSelect
                value={currency}
                onChange={setCurrency}
                mono
                options={currencyOptionsFor(currency)}
              />
            </label>

            <label className="flex flex-col gap-1 sm:col-span-3">
              <span className="text-[10px] uppercase font-mono text-neutral-500">Date / Time (Optional)</span>
              <DateTimePicker
                value={transactionDate}
                onChange={setTransactionDate}
                placeholder="Now (current time)"
                now={nowInZone(timezone)}
              />
            </label>
          </div>

          <div className="grid grid-cols-1 sm:grid-cols-12 gap-3">
            <input
              type="text"
              placeholder="Note (optional)"
              value={note}
              onChange={(e) => setNote(e.target.value)}
              className="sm:col-span-6 h-9 px-3 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 focus:outline-none focus:border-neutral-500 font-sans"
            />
            <button
              type="submit"
              disabled={buyMutation.isPending || recordMutation.isPending}
              className="sm:col-span-3 h-9 inline-flex items-center justify-center gap-1.5 px-3 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50 cursor-pointer"
              title="Buy position and deduct cost from cash balance"
            >
              <Plus className="w-3.5 h-3.5" />
              <span>{buyMutation.isPending ? 'Buying...' : 'Buy'}</span>
            </button>
            <button
              type="button"
              disabled={buyMutation.isPending || recordMutation.isPending}
              onClick={handleRecord}
              className="sm:col-span-3 h-9 inline-flex items-center justify-center gap-1.5 px-3 rounded-md bg-neutral-100 dark:bg-neutral-800 text-neutral-700 dark:text-neutral-200 border border-neutral-200 dark:border-neutral-700 text-xs font-medium hover:bg-neutral-200 dark:hover:bg-neutral-700 transition disabled:opacity-50 cursor-pointer"
              title="Record already-owned stock or crypto without deducting cash balance"
            >
              <BookmarkPlus className="w-3.5 h-3.5" />
              <span>{recordMutation.isPending ? 'Recording...' : 'Record Existing (No Cash)'}</span>
            </button>
          </div>
        </form>
      </div>

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
      <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4">
        <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3 flex-wrap gap-2">
          <div className="flex items-center gap-3">
            <span className="text-xs font-mono text-neutral-500 dark:text-neutral-400">HOLDINGS</span>
            <span className="text-xs font-mono text-neutral-500">{(holdings ?? []).length} positions</span>
          </div>

          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() =>
                triggerAiPrompt(
                  'Review my investment portfolio asset allocation, performance, and cash balances. Provide a concise executive overview and highlight key insights.'
                )
              }
              className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-neutral-800 dark:text-neutral-200 rounded-lg hover:bg-neutral-100 dark:hover:bg-neutral-800 transition cursor-pointer shadow-2xs"
              title="Ask AI for portfolio review"
            >
              <Sparkles className="w-3.5 h-3.5 text-neutral-500" />
              <span>AI Review</span>
            </button>

            <button
              type="button"
              onClick={() => setAlertsModalOpen(true)}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-neutral-800 dark:text-neutral-200 rounded-lg hover:bg-neutral-100 dark:hover:bg-neutral-800 transition cursor-pointer shadow-2xs"
              title="Manage market price and volatility alerts"
            >
              <Bell className="w-3.5 h-3.5 text-neutral-500" />
              <span>Price Alerts</span>
            </button>

            <button
              type="button"
              onClick={() => refreshPricesMutation.mutate()}
              disabled={refreshPricesMutation.isPending || (holdings ?? []).length === 0}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-neutral-800 dark:text-neutral-200 rounded-lg hover:bg-neutral-100 dark:hover:bg-neutral-800 transition disabled:opacity-50 cursor-pointer shadow-2xs"
              title="Automatically fetch latest market prices for all holdings using free Yahoo Finance and crypto feeds"
            >
              <RefreshCw className={`w-3.5 h-3.5 ${refreshPricesMutation.isPending ? 'animate-spin' : ''}`} />
              <span>{refreshPricesMutation.isPending ? 'Updating...' : 'Refresh Prices'}</span>
            </button>
          </div>
        </div>

        {refreshStatus && (
          <div className="p-2.5 rounded-lg bg-neutral-100 dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-neutral-700 dark:text-neutral-300 text-xs font-mono">
            {refreshStatus}
          </div>
        )}

        {isLoading ? (
          <div className="py-8 text-center text-xs font-mono text-neutral-500">Loading holdings...</div>
        ) : (holdings ?? []).length === 0 ? (
          <div className="py-8 text-center text-xs font-mono text-neutral-500 border border-dashed border-neutral-300 dark:border-neutral-800 rounded">
            No holdings yet.
          </div>
        ) : (
          <div className="divide-y divide-neutral-200 dark:divide-neutral-850">
            {(holdings ?? []).map((h) => {
              const marketValue =
                h.current_price_minor != null
                  ? Math.floor((h.quantity_minor * h.current_price_minor) / 10 ** QUANTITY_EXPONENT)
                  : null;
              const costBasis = Math.floor(
                (h.quantity_minor * h.avg_cost_minor) / 10 ** QUANTITY_EXPONENT
              );
              const unrealizedPl = marketValue != null ? marketValue - costBasis : null;

              if (sellingId === h.id) {
                return (
                  <SellRow
                    key={h.id}
                    holding={h}
                    timezone={timezone}
                    saving={sellMutation.isPending}
                    onCancel={() => setSellingId(null)}
                    onSell={async (qty, sellPrice, executedAt) => {
                      await sellMutation.mutateAsync({
                        id: h.id,
                        quantity: qty,
                        price: sellPrice,
                        executed_at: executedAt,
                      });
                      setSellingId(null);
                    }}
                  />
                );
              }

              return (
                <div
                  key={h.id}
                  data-testid="holding-row"
                  className="py-3 flex items-center justify-between gap-4 px-2 rounded transition hover:bg-neutral-100 dark:hover:bg-neutral-900/30"
                >
                  <div className="space-y-0.5 min-w-0">
                    <div className="flex items-center gap-2 flex-wrap">
                      <p className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">{h.symbol}</p>
                      <button
                        type="button"
                        onClick={() => {
                          setEditingSymbolHolding(h);
                          setNewSymbolInput(h.symbol);
                          setSymbolError(null);
                        }}
                        title="Edit symbol"
                        className="p-1 rounded text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-200 dark:hover:bg-neutral-800 transition cursor-pointer"
                      >
                        <Pencil className="w-3 h-3" />
                      </button>
                      {failedSymbols.has(h.symbol) && (
                        <span
                          className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-mono bg-amber-50 dark:bg-amber-950/50 text-amber-700 dark:text-amber-400 border border-amber-200 dark:border-amber-800/60"
                          title={`No market quote found for "${h.symbol}". Click the pencil to correct the symbol or enter price manually.`}
                        >
                          <AlertTriangle className="w-3 h-3 text-amber-600 dark:text-amber-400 shrink-0" />
                          Quote not found
                        </span>
                      )}
                      <span className="text-xs font-mono text-neutral-500">
                        {formatQuantity(h.quantity_minor)} @ avg {formatMinor(h.avg_cost_minor, h.currency_exponent)}{' '}
                        {h.currency}
                      </span>
                    </div>
                    {editingNoteId === h.id ? (
                      <NoteRow
                        holding={h}
                        saving={noteMutation.isPending}
                        onCancel={() => setEditingNoteId(null)}
                        onSave={async (newNote) => {
                          await noteMutation.mutateAsync({ id: h.id, note: newNote });
                          setEditingNoteId(null);
                        }}
                      />
                    ) : (
                      <button
                        type="button"
                        onClick={() => setEditingNoteId(h.id)}
                        className="flex items-center gap-1 text-xs text-neutral-500 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200 transition max-w-md"
                      >
                        <span className="truncate">{h.note || 'add a note'}</span>
                        <Pencil className="w-3 h-3 shrink-0" />
                      </button>
                    )}
                    {editingPriceId === h.id ? (
                      <PriceRow
                        holding={h}
                        saving={priceMutation.isPending}
                        onCancel={() => setEditingPriceId(null)}
                        onSave={async (newPrice) => {
                          await priceMutation.mutateAsync({ id: h.id, price: newPrice });
                          setEditingPriceId(null);
                        }}
                      />
                    ) : (
                      <button
                        type="button"
                        onClick={() => setEditingPriceId(h.id)}
                        className="inline-flex items-center gap-1 text-[11px] font-mono text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-200 transition"
                      >
                        <Tag className="w-3 h-3" />
                        {h.current_price_minor != null
                          ? `price ${formatMinor(h.current_price_minor, h.currency_exponent)} ${h.currency}`
                          : 'set current price'}
                        <Pencil className="w-3 h-3" />
                      </button>
                    )}
                    {h.price_updated_at && (
                      <span className="text-[10px] text-neutral-400 dark:text-neutral-500 font-mono ml-1">
                        · {new Date(h.price_updated_at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
                      </span>
                    )}
                  </div>

                  <div className="flex items-center gap-3 shrink-0">
                    {marketValue != null && unrealizedPl != null ? (
                      <div className="text-right">
                        <div className="font-mono text-sm font-medium text-neutral-900 dark:text-neutral-100">
                          {formatMinor(marketValue, h.currency_exponent)} {h.currency}
                        </div>
                        <div className={`font-mono text-xs ${plColor(unrealizedPl)}`}>
                          {formatSigned(unrealizedPl, h.currency_exponent)} {h.currency}
                        </div>
                      </div>
                    ) : (
                      <span className="text-xs font-mono text-neutral-400">no price set</span>
                    )}
                    <button
                      onClick={() => singleRefreshMutation.mutate(h.id)}
                      disabled={singleRefreshMutation.isPending && refreshingHoldingId === h.id}
                      title="Fetch latest market price"
                      className="p-1.5 rounded text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-200 dark:hover:bg-neutral-900 transition disabled:opacity-50 cursor-pointer"
                    >
                      <RefreshCw
                        className={`w-4 h-4 ${
                          singleRefreshMutation.isPending && refreshingHoldingId === h.id
                            ? 'animate-spin text-neutral-500'
                            : ''
                        }`}
                      />
                    </button>
                    <button
                      onClick={() => {
                        setAlertSymbol(h.symbol);
                        setAlertCurrency(h.currency);
                        setAlertsModalOpen(true);
                      }}
                      title={`Set price alert for ${h.symbol}`}
                      className="p-1.5 rounded text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-200 dark:hover:bg-neutral-900 transition cursor-pointer"
                    >
                      <Bell className="w-4 h-4" />
                    </button>
                    <button
                      onClick={() => setSellingId(h.id)}
                      title="Sell"
                      className="p-1.5 rounded text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-200 dark:hover:bg-neutral-900 transition"
                    >
                      <ArrowDownRight className="w-4 h-4" />
                    </button>
                    <button
                      onClick={() => setRemovingHolding(h)}
                      title="Remove holding"
                      className="p-1.5 rounded text-neutral-400 hover:text-rose-600 dark:hover:text-rose-400 hover:bg-neutral-200 dark:hover:bg-neutral-900 transition cursor-pointer"
                    >
                      <Trash2 className="w-4 h-4" />
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* Edit Symbol Modal */}
      {editingSymbolHolding && (
        <div className="fixed inset-0 z-50 bg-black/50 flex items-center justify-center p-4">
          <div className="bg-white dark:bg-neutral-900 rounded-lg border border-neutral-200 dark:border-neutral-800 p-5 max-w-sm w-full space-y-4 shadow-xl">
            <div>
              <h3 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
                Edit Ticker Symbol
              </h3>
              <p className="text-xs text-neutral-500 mt-0.5">
                Update the ticker symbol for this position. Position quantity and cost basis will remain unchanged.
              </p>
            </div>

            {symbolError && (
              <div className="flex items-center gap-2 p-2.5 rounded bg-red-50 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 text-red-700 dark:text-red-300 text-xs">
                <AlertCircle className="w-3.5 h-3.5 shrink-0" />
                <span>{symbolError}</span>
              </div>
            )}

            <div className="space-y-1">
              <label className="text-[10px] uppercase font-mono text-neutral-500">
                New Symbol
              </label>
              <TickerSearchInput
                value={newSymbolInput}
                onChange={setNewSymbolInput}
                placeholder="e.g. AAPL, BTC..."
                className="px-3 py-2 rounded bg-neutral-50 dark:bg-neutral-800 border border-neutral-300 dark:border-neutral-700 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-mono uppercase"
              />
            </div>

            <div className="flex items-center justify-end gap-2 pt-2 border-t border-neutral-100 dark:border-neutral-800">
              <button
                type="button"
                onClick={() => {
                  setEditingSymbolHolding(null);
                  setSymbolError(null);
                }}
                className="px-3 py-1.5 rounded text-xs text-neutral-600 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200 cursor-pointer"
              >
                Cancel
              </button>
              <button
                type="button"
                disabled={!newSymbolInput.trim() || updateSymbolMutation.isPending}
                onClick={() => {
                  if (editingSymbolHolding && newSymbolInput.trim()) {
                    updateSymbolMutation.mutate({
                      id: editingSymbolHolding.id,
                      symbol: newSymbolInput.trim(),
                    });
                  }
                }}
                className="px-3.5 py-1.5 rounded bg-neutral-900 dark:bg-neutral-100 text-white dark:text-black text-xs font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-50 cursor-pointer"
              >
                {updateSymbolMutation.isPending ? 'Saving...' : 'Save Symbol'}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Remove Holding Confirmation Modal */}
      {removingHolding && (
        <div className="fixed inset-0 z-50 bg-black/50 flex items-center justify-center p-4">
          <div className="bg-white dark:bg-neutral-900 rounded-lg border border-neutral-200 dark:border-neutral-800 p-5 max-w-sm w-full space-y-4 shadow-xl">
            <div className="flex items-start gap-3">
              <div className="p-2 rounded-full bg-rose-100 dark:bg-rose-950/60 text-rose-600 dark:text-rose-400 shrink-0">
                <Trash2 className="w-5 h-5" />
              </div>
              <div className="space-y-1">
                <h3 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
                  Remove {removingHolding.symbol}?
                </h3>
                <p className="text-xs text-neutral-600 dark:text-neutral-400 leading-relaxed">
                  This will delete this position from your active portfolio. This action is for administrative corrections and <span className="font-semibold text-neutral-800 dark:text-neutral-200">will not credit your cash balance</span>.
                </p>
                <p className="text-[11px] text-neutral-500 dark:text-neutral-400 pt-1">
                  To sell your shares and receive cash proceeds, click <span className="font-medium text-neutral-700 dark:text-neutral-300">Sell (↘)</span> instead.
                </p>
              </div>
            </div>

            <div className="flex items-center justify-end gap-2 pt-2 border-t border-neutral-100 dark:border-neutral-800">
              <button
                type="button"
                onClick={() => setRemovingHolding(null)}
                className="px-3 py-1.5 rounded text-xs text-neutral-600 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-200 cursor-pointer"
              >
                Cancel
              </button>
              <button
                type="button"
                disabled={deleteMutation.isPending}
                onClick={async () => {
                  await deleteMutation.mutateAsync(removingHolding.id);
                  setRemovingHolding(null);
                }}
                className="px-3.5 py-1.5 rounded bg-rose-600 hover:bg-rose-700 text-white text-xs font-medium transition disabled:opacity-50 cursor-pointer"
              >
                {deleteMutation.isPending ? 'Removing...' : 'Remove Position'}
              </button>
            </div>
          </div>
        </div>
      )}

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
    }
    if (event.type === 'PortfolioCashUpdated') {
      queryClient.invalidateQueries({ queryKey: ['portfolio-cash'] });
      queryClient.invalidateQueries({ queryKey: ['portfolio-summary'] });
      queryClient.invalidateQueries({ queryKey: ['portfolio', 'activity'] });
      queryClient.invalidateQueries({ queryKey: ['portfolio-history'] });
    }
  },
};
