import React, { useState } from 'react';
import {
  Sparkles,
  Bell,
  RefreshCw,
  Pencil,
  AlertTriangle,
  Tag,
  ArrowDownRight,
  Trash2,
  X,
  Check,
} from 'lucide-react';
import { Holding } from './types';
import { formatShortTimeInZone } from '../../core/settings';
import { formatMinor, formatQuantity, formatSigned, plColor, QUANTITY_EXPONENT } from './utils';
import { SellRow } from './SellRow';
import { EditSymbolModal, RemoveHoldingModal } from './HoldingModals';
import { NoteRow, PriceRow } from './InlineRows';

export interface HoldingTableProps {
  holdings?: Holding[];
  isLoading: boolean;
  failedSymbols: Set<string>;
  timezone: string;
  refreshStatus: string | null;
  refreshingHoldingId: number | null;
  isRefreshingPrices: boolean;
  onRefreshPrices: () => void;
  onSingleRefresh: (id: number) => void;
  onSell: (id: number, qty: string, price: string, executedAt?: string) => Promise<void>;
  onSavePrice: (id: number, price: string) => Promise<void>;
  onSaveNote: (id: number, note: string) => Promise<void>;
  onSaveSymbol: (id: number, symbol: string) => Promise<void>;
  onDeleteHolding: (id: number) => Promise<void>;
  onOpenAlerts: (symbol: string, currency: string) => void;
  onAiReview: () => void;
  isSelling: boolean;
  isSavingPrice: boolean;
  isSavingNote: boolean;
  isSavingSymbol: boolean;
  isDeleting: boolean;
}

export const HoldingTable: React.FC<HoldingTableProps> = ({
  holdings,
  isLoading,
  failedSymbols,
  timezone,
  refreshStatus,
  refreshingHoldingId,
  isRefreshingPrices,
  onRefreshPrices,
  onSingleRefresh,
  onSell,
  onSavePrice,
  onSaveNote,
  onSaveSymbol,
  onDeleteHolding,
  onOpenAlerts,
  onAiReview,
  isSelling,
  isSavingPrice,
  isSavingNote,
  isSavingSymbol,
  isDeleting,
}) => {
  const [sellingId, setSellingId] = useState<number | null>(null);
  const [editingPriceId, setEditingPriceId] = useState<number | null>(null);
  const [editingNoteId, setEditingNoteId] = useState<number | null>(null);
  const [editingSymbolHolding, setEditingSymbolHolding] = useState<Holding | null>(null);
  const [newSymbolInput, setNewSymbolInput] = useState('');
  const [symbolError, setSymbolError] = useState<string | null>(null);
  const [removingHolding, setRemovingHolding] = useState<Holding | null>(null);

  return (
    <div className="rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 p-5 space-y-4">
      <div className="flex items-center justify-between border-b border-neutral-200 dark:border-neutral-800 pb-3 flex-wrap gap-2">
        <div className="flex items-center gap-3">
          <span className="text-xs font-mono text-neutral-500 dark:text-neutral-400">HOLDINGS</span>
          <span className="text-xs font-mono text-neutral-500">{(holdings ?? []).length} positions</span>
        </div>

        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={onAiReview}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-neutral-800 dark:text-neutral-200 rounded-lg hover:bg-neutral-100 dark:hover:bg-neutral-800 transition cursor-pointer shadow-2xs"
            title="Ask AI for portfolio review"
          >
            <Sparkles className="w-3.5 h-3.5 text-neutral-500" />
            <span>AI Review</span>
          </button>

          <button
            type="button"
            onClick={() => onOpenAlerts('', 'USD')}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-neutral-800 dark:text-neutral-200 rounded-lg hover:bg-neutral-100 dark:hover:bg-neutral-800 transition cursor-pointer shadow-2xs"
            title="Manage market price and volatility alerts"
          >
            <Bell className="w-3.5 h-3.5 text-neutral-500" />
            <span>Price Alerts</span>
          </button>

          <button
            type="button"
            onClick={onRefreshPrices}
            disabled={isRefreshingPrices || (holdings ?? []).length === 0}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-neutral-800 dark:text-neutral-200 rounded-lg hover:bg-neutral-100 dark:hover:bg-neutral-800 transition disabled:opacity-50 cursor-pointer shadow-2xs"
            title="Automatically fetch latest market prices for all holdings using free Yahoo Finance and crypto feeds"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${isRefreshingPrices ? 'animate-spin' : ''}`} />
            <span>{isRefreshingPrices ? 'Updating...' : 'Refresh Prices'}</span>
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
                  saving={isSelling}
                  onCancel={() => setSellingId(null)}
                  onSell={async (qty, sellPrice, executedAt) => {
                    await onSell(h.id, qty, sellPrice, executedAt);
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
                      saving={isSavingNote}
                      onCancel={() => setEditingNoteId(null)}
                      onSave={async (newNote) => {
                        await onSaveNote(h.id, newNote);
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
                      saving={isSavingPrice}
                      onCancel={() => setEditingPriceId(null)}
                      onSave={async (newPrice) => {
                        await onSavePrice(h.id, newPrice);
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
                      · {formatShortTimeInZone(h.price_updated_at, timezone)}
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
                    onClick={() => onSingleRefresh(h.id)}
                    disabled={refreshingHoldingId === h.id}
                    title="Fetch latest market price"
                    className="p-1.5 rounded text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-200 dark:hover:bg-neutral-900 transition disabled:opacity-50 cursor-pointer"
                  >
                    <RefreshCw
                      className={`w-4 h-4 ${
                        refreshingHoldingId === h.id ? 'animate-spin text-neutral-500' : ''
                      }`}
                    />
                  </button>
                  <button
                    onClick={() => onOpenAlerts(h.symbol, h.currency)}
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

      {/* Edit Symbol Modal */}
      {editingSymbolHolding && (
        <EditSymbolModal
          holding={editingSymbolHolding}
          symbolInput={newSymbolInput}
          onChangeSymbolInput={setNewSymbolInput}
          error={symbolError}
          isSaving={isSavingSymbol}
          onClose={() => {
            setEditingSymbolHolding(null);
            setSymbolError(null);
          }}
          onSave={async () => {
            if (editingSymbolHolding && newSymbolInput.trim()) {
              try {
                await onSaveSymbol(editingSymbolHolding.id, newSymbolInput.trim());
                setEditingSymbolHolding(null);
                setNewSymbolInput('');
                setSymbolError(null);
              } catch (err) {
                setSymbolError(err instanceof Error ? err.message : 'Failed to save symbol');
              }
            }
          }}
        />
      )}

      {/* Remove Holding Confirmation Modal */}
      {removingHolding && (
        <RemoveHoldingModal
          holding={removingHolding}
          isDeleting={isDeleting}
          onClose={() => setRemovingHolding(null)}
          onConfirm={async () => {
            await onDeleteHolding(removingHolding.id);
            setRemovingHolding(null);
          }}
        />
      )}
    </div>
  );
};
