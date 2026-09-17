export interface MonthlyCashflowBucket {
  month: string; // "YYYY-MM"
  income_minor: number;
  expense_minor: number;
  net_minor: number;
  currency: string;
  currency_exponent: number;
}

export interface ExpenseCategorySlice {
  category_name: string;
  amount_minor: number;
  percentage: number;
  tx_count: number;
  currency: string;
  currency_exponent: number;
}

export interface FinanceAnalyticsReport {
  currency: string;
  currency_exponent: number;
  months_requested: number;
  total_income_minor: number;
  total_expense_minor: number;
  net_savings_minor: number;
  monthly_cashflow: MonthlyCashflowBucket[];
  expense_categories: ExpenseCategorySlice[];
}

export interface FinanceShare {
  owner_id: string;
  token: string;
  is_enabled: boolean;
  title: string | null;
  created_at?: string | null;
  updated_at?: string | null;
}

export interface PublicTransaction {
  id: number;
  direction: string;
  amount_minor: number;
  currency: string;
  currency_exponent: number;
  category_name: string | null;
  note: string | null;
  occurred_at: string;
}

export interface PublicFinanceReport {
  title: string;
  currency: string;
  currency_exponent: number;
  total_income_minor: number;
  total_expense_minor: number;
  net_savings_minor: number;
  monthly_cashflow: MonthlyCashflowBucket[];
  expense_categories: ExpenseCategorySlice[];
  recent_transactions: PublicTransaction[];
}

export interface CurrencyNetWorthBreakdown {
  currency: string;
  currency_exponent: number;
  ledger_cash_minor: number;
  portfolio_market_value_minor: number;
  portfolio_cash_minor: number;
  converted_total_minor: number;
}

export interface NetWorthResponse {
  primary_currency: string;
  primary_currency_exponent: number;
  total_net_worth_minor: number;
  ledger_cash_minor: number;
  portfolio_market_value_minor: number;
  portfolio_cash_minor: number;
  fx_source: string;
  native_breakdowns: CurrencyNetWorthBreakdown[];
}

export interface ReceiptExtraction {
  merchant?: string | null;
  amount?: string | null;
  currency?: string | null;
  date?: string | null;
  category_suggestion?: string | null;
}

export interface OcrTextBlock {
  text: string;
  score: number;
}

export interface OcrScanResponse {
  raw_text: string;
  blocks: OcrTextBlock[];
  structured?: ReceiptExtraction | null;
}



