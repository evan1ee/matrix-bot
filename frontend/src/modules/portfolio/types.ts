import type { components } from '../../types/api';

export type Holding = components['schemas']['Holding'];

export type PortfolioCurrencySummary = components['schemas']['PortfolioCurrencySummary'];

export type SellResponse = components['schemas']['SellResponse'];

export type CashBalance = components['schemas']['CashBalance'];

export type RefreshSummary = components['schemas']['RefreshSummary'];

// Server types `action` as a bare `string` (no utoipa enum annotation), but
// every write site (backend/src/modules/portfolio/{trade,cash,audit}.rs and
// accounts/merge.rs) only ever uses these six values. Narrowed here rather
// than widened, so `entry.action === 'merge'` checks stay meaningful.
// Backend finding: PortfolioAuditEntry.action should be a proper OpenAPI enum.
export type PortfolioAuditEntry = Omit<components['schemas']['PortfolioAuditEntry'], 'action'> & {
  action: 'buy' | 'sell' | 'deposit' | 'withdraw' | 'record' | 'merge';
};

export type AuditVerificationResult = components['schemas']['AuditVerificationResult'];

export type TickerSearchResult = components['schemas']['TickerSearchResult'];

export type AlertCondition = components['schemas']['AlertCondition'];

// Server types `condition` as a bare `string` even though the dedicated
// `AlertCondition` enum schema exists and is used for the request type
// (CreateAlertRequest) — just not wired up on this response field. Narrowed
// here rather than widened.
// Backend finding: PriceAlert.condition should reuse the AlertCondition enum.
export type PriceAlert = Omit<components['schemas']['PriceAlert'], 'condition'> & {
  condition: AlertCondition;
};

export type PortfolioHistoryPoint = components['schemas']['PortfolioHistoryPoint'];

export type PortfolioHistoryResponse = components['schemas']['PortfolioHistoryResponse'];
