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
