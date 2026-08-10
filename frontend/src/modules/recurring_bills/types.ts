export type BillCategory = 'subscription' | 'fixed_bill' | 'installment' | 'recurring_transfer';
export type BillFrequency = 'daily' | 'weekly' | 'monthly' | 'quarterly' | 'yearly';
export type BillStatus = 'active' | 'paused' | 'completed';

export interface RecurringBill {
  id: number;
  room_id: string;
  sender_mxid: string;
  sender_name: string;
  name: string;
  category: BillCategory | string;
  amount_minor: number;
  currency: string;
  currency_exponent: number;
  frequency: BillFrequency | string;
  due_day?: number | null;
  next_due_at: string;
  total_installments?: number | null;
  paid_installments: number;
  auto_pay: boolean;
  status: BillStatus | string;
  notes?: string | null;
  created_at?: string | null;
  updated_at?: string | null;
}

export interface CreateBillPayload {
  name: string;
  category: string;
  amount_minor: number;
  currency: string;
  currency_exponent?: number;
  frequency?: string;
  due_day?: number;
  next_due_at: string;
  total_installments?: number;
  auto_pay?: boolean;
  notes?: string;
}

export interface UpdateBillPayload {
  name?: string;
  category?: string;
  amount_minor?: number;
  currency?: string;
  frequency?: string;
  due_day?: number;
  next_due_at?: string;
  total_installments?: number;
  paid_installments?: number;
  auto_pay?: boolean;
  status?: string;
  notes?: string;
}
