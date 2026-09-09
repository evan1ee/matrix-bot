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
  prev_hash: string;
  entry_hash: string;
  created_at?: string | null;
}

export interface AuditVerificationResult {
  is_valid: boolean;
  total_verified: number;
  genesis_hash: string;
  head_hash?: string | null;
  broken_entry_id?: number | null;
  message: string;
}

export interface TickerSearchResult {
  symbol: string;
  name: string;
  exchange?: string | null;
  quote_type?: string | null;
}

export type AlertCondition = 'above' | 'below';

export interface PriceAlert {
  id: number;
  owner_id: string;
  symbol: string;
  target_price_minor: number;
  currency: string;
  currency_exponent: number;
  condition: AlertCondition;
  is_triggered: boolean;
  triggered_at?: string | null;
  created_at: string;
}

export interface PortfolioHistoryPoint {
  date: string;
  holdings_value_minor: number;
  cash_minor: number;
  total_net_worth_minor: number;
}

export interface PortfolioHistoryResponse {
  currency: string;
  currency_exponent: number;
  timeframe: string;
  points: PortfolioHistoryPoint[];
  starting_value_minor: number;
  current_value_minor: number;
  period_change_minor: number;
  period_change_pct: number;
}

