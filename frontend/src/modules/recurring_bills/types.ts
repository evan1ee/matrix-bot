import type { components } from '../../types/api';

// Frontend-only presentation unions: the server types category/frequency/status
// as bare `string` fields (validated server-side, not schema enums). Used
// below in RecurringBill as `X | string` so a value outside this list still
// type-checks — the server is the source of truth for what's actually valid.
export type BillCategory = 'subscription' | 'fixed_bill' | 'installment' | 'recurring_transfer';
export type BillFrequency = 'daily' | 'weekly' | 'monthly' | 'quarterly' | 'yearly';
export type BillStatus = 'active' | 'paused' | 'completed';

export type RecurringBill = components['schemas']['RecurringBill'];

export type CreateBillPayload = components['schemas']['CreateBillPayload'];

export type UpdateBillPayload = components['schemas']['UpdateBillPayload'];
