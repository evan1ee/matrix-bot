import type { components } from '../../types/api';

export type MonthlyCashflowBucket = components['schemas']['MonthlyCashflowBucket'];

export type ExpenseCategorySlice = components['schemas']['ExpenseCategorySlice'];

export type FinanceAnalyticsReport = components['schemas']['FinanceAnalyticsReport'];

export type FinanceShare = components['schemas']['FinanceShare'];

export type PublicTransaction = components['schemas']['PublicTransaction'];

export type PublicFinanceReport = components['schemas']['PublicFinanceReport'];

// Server field names: cash_deposits_minor, fx_rate_to_primary, converted_minor,
// total_minor (see backend/src/modules/finance/net_worth.rs). The old
// hand-written type used portfolio_cash_minor/converted_total_minor, which
// never existed on the server — NetWorthStatStrip was reading undefined at
// runtime. Fixed on the frontend below to use the real field names.
export type CurrencyNetWorthBreakdown = components['schemas']['CurrencyNetWorthBreakdown'];

// Server field names: cash_deposits_minor, fx_rates (a currency->rate map),
// currencies (the per-currency breakdown list). The old hand-written type had
// portfolio_cash_minor, fx_source (string) and native_breakdowns, none of
// which the server has ever returned. Fixed on the frontend below.
export type NetWorthResponse = components['schemas']['NetWorthResponse'];

// Frontend-only name; server schema is ReceiptExtractionDto.
export type ReceiptExtraction = components['schemas']['ReceiptExtractionDto'];

// Frontend-only name; server schema is OcrBlockDto.
export type OcrTextBlock = components['schemas']['OcrBlockDto'];

export type OcrScanResponse = components['schemas']['OcrScanResponse'];

export type ReceiptUploadResponse = components['schemas']['ReceiptUploadResponse'];

export type Direction = 'expense' | 'income';

export interface Transaction {
  id: number;
  room_id: string;
  owner_id: string;
  sender_name: string;
  direction: string;
  /** Minor units — always positive; `direction` carries the sign. */
  amount_minor: number;
  currency: string;
  currency_exponent: number;
  category_id?: number | null;
  category_name?: string | null;
  note?: string | null;
  occurred_at: string;
  created_at?: string | null;
  source_kind?: string | null;
  source_id?: number | null;
  /** Set when a receipt file is attached; view it with the receipt-url endpoint. */
  receipt_storage_key?: string | null;
}

export interface Category {
  id: number;
  owner_id: string;
  name: string;
  created_at?: string | null;
}

export interface LedgerResponse {
  items: Transaction[];
  page: number;
  limit: number;
  total: number;
}

export interface CategoryTotal {
  category: string | null;
  expense_minor: number;
  income_minor: number;
}

export interface CurrencySummary {
  currency: string;
  currency_exponent: number;
  expense_minor: number;
  income_minor: number;
  net_minor: number;
  categories: CategoryTotal[];
}

export type BudgetStatus = components["schemas"]["BudgetStatusDto"];

export type BudgetState = components["schemas"]["BudgetStateDto"];

export type CreateBudgetBody = components["schemas"]["CreateBudgetRequest"];

export type UpdateBudgetBody = components["schemas"]["UpdateBudgetRequest"];
