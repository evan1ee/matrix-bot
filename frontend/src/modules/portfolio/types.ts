export interface Holding {
  id: number;
  owner_id: string;
  symbol: string;
  /** Minor units, scaled at `QUANTITY_EXPONENT` — always positive. */
  quantity_minor: number;
  /** Minor units, scaled at `currency_exponent`. */
  avg_cost_minor: number;
  currency: string;
  currency_exponent: number;
  current_price_minor?: number | null;
  price_updated_at?: string | null;
  note?: string | null;
  created_at?: string | null;
  updated_at?: string | null;
}

export interface PortfolioCurrencySummary {
  currency: string;
  currency_exponent: number;
  cost_basis_minor: number;
  market_value_minor: number;
  unrealized_pl_minor: number;
  priced_holdings: number;
  unpriced_holdings: number;
  cash_minor: number;
}

export interface SellResponse {
  holding: Holding | null;
  realized_pl_minor: number;
}

export interface CashBalance {
  id: number;
  owner_id: string;
  currency: string;
  currency_exponent: number;
  balance_minor: number;
  updated_at?: string | null;
}

export interface RefreshSummary {
  updated: string[];
  failed: string[];
}

export interface PortfolioAuditEntry {
  id: number;
  owner_id: string;
  action: 'buy' | 'sell' | 'deposit' | 'withdraw' | 'record';
  symbol?: string | null;
  quantity_minor?: number | null;
  price_minor?: number | null;
  amount_minor: number;
  currency: string;
  currency_exponent: number;
  realized_pl_minor?: number | null;
  note?: string | null;
  created_at?: string | null;
}

export interface TickerSearchResult {
  symbol: string;
  name: string;
  exchange?: string | null;
  quote_type?: string | null;
}
