export type BillCategory = 'subscription' | 'fixed_bill' | 'installment' | 'recurring_transfer';
export type BillFrequency = 'daily' | 'weekly' | 'monthly' | 'quarterly' | 'yearly';
export type BillStatus = 'active' | 'paused' | 'completed';

export interface RecurringBill {
  id: number;
  room_id: string;
  owner_id: string;
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
  /** Minutes before `next_due_at` to send a reminder; null means none. */
  remind_before_minutes?: number | null;
  /** Matrix room the reminder is delivered to. */
  remind_room_id?: string | null;
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
  remind_before_minutes?: number;
  remind_room_id?: string;
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
  /** `null` clears the lead time; omitting it leaves the current one alone. */
  remind_before_minutes?: number | null;
  remind_room_id?: string;
}
