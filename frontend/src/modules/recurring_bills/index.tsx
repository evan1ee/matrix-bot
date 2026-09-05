import React, { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import { FrontendModule } from '../../core/types';
import { useSettings } from '../../core/settings';
import {
  RefreshCcw,
  Plus,
  Calendar,
  CreditCard,
  Building,
  ArrowRightLeft,
  CheckCircle2,
  Trash2,
  Pencil,
  Zap,
  BellRing,
} from 'lucide-react';
import { BillModal } from './BillModal';
import {
  BillCategory,
  CreateBillPayload,
  RecurringBill,
  UpdateBillPayload,
} from './types';

const QUERY_KEY = ['recurring-bills'];

function getCategoryBadge(category: BillCategory | string) {
  switch (category) {
    case 'subscription':
      return {
        label: 'Subscription',
        icon: CreditCard,
      };
    case 'fixed_bill':
      return {
        label: 'Fixed Bill',
        icon: Building,
      };
    case 'installment':
      return {
        label: 'Installment',
        icon: Calendar,
      };
    case 'recurring_transfer':
      return {
        label: 'Transfer',
        icon: ArrowRightLeft,
      };
    default:
      return {
        label: category,
        icon: RefreshCcw,
      };
  }
}

export const RecurringBillsView: React.FC = () => {
  const queryClient = useQueryClient();
  const settings = useSettings();
  const displayCurrency = settings?.currency || 'USD';

  const [activeTab, setActiveTab] = useState<'all' | BillCategory>('all');
  const [isModalOpen, setIsModalOpen] = useState(false);
  const [editingBill, setEditingBill] = useState<RecurringBill | null>(null);
  const [formError, setFormError] = useState<string | null>(null);

  const { data: bills = [], isLoading } = useQuery<RecurringBill[]>({
    queryKey: QUERY_KEY,
    queryFn: () => fetchApi<RecurringBill[]>('/recurring-bills'),
  });

  const createMutation = useMutation({
    mutationFn: (payload: CreateBillPayload) =>
      fetchApi<RecurringBill>('/recurring-bills', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEY });
      setIsModalOpen(false);
      setFormError(null);
    },
    onError: (err: Error) => {
      setFormError(err.message || 'Failed to create item');
    },
  });

  const updateMutation = useMutation({
    mutationFn: ({ id, payload }: { id: number; payload: UpdateBillPayload }) =>
      fetchApi<RecurringBill>(`/recurring-bills/${id}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEY });
      setIsModalOpen(false);
      setEditingBill(null);
      setFormError(null);
    },
    onError: (err: Error) => {
      setFormError(err.message || 'Failed to update item');
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: number) =>
      fetchApi(`/recurring-bills/${id}`, { method: 'DELETE' }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEY });
    },
  });

  const payMutation = useMutation({
    mutationFn: (id: number) =>
      fetchApi<RecurringBill>(`/recurring-bills/${id}/pay`, { method: 'POST' }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEY });
    },
  });

  const filteredBills = bills.filter(
    (b) => activeTab === 'all' || b.category === activeTab
  );

  // Statistics calculation
  const totalMonthlyCommitment = bills.reduce((sum, b) => {
    let monthlyAmount = b.amount_minor / 100;
    if (b.frequency === 'yearly') monthlyAmount /= 12;
    else if (b.frequency === 'weekly') monthlyAmount *= 4.33;
    else if (b.frequency === 'quarterly') monthlyAmount /= 3;
    else if (b.frequency === 'daily') monthlyAmount *= 30;
    return sum + monthlyAmount;
  }, 0);

  const subscriptionsCount = bills.filter((b) => b.category === 'subscription').length;
  const fixedBillsCount = bills.filter((b) => b.category === 'fixed_bill').length;
  const installmentsCount = bills.filter((b) => b.category === 'installment').length;
  const transfersCount = bills.filter((b) => b.category === 'recurring_transfer').length;

  const handleModalSubmit = (payload: CreateBillPayload | UpdateBillPayload) => {
    if (editingBill) {
      updateMutation.mutate({ id: editingBill.id, payload: payload as UpdateBillPayload });
    } else {
      createMutation.mutate(payload as CreateBillPayload);
    }
  };

  return (
    <div className="space-y-6">
      {/* Vercel Editorial Header */}
      <div className="flex flex-col sm:flex-row sm:items-baseline justify-between gap-4 border-b border-neutral-200 dark:border-neutral-800 pb-5">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight text-neutral-900 dark:text-white flex items-center gap-2">
            <RefreshCcw className="w-5 h-5 text-neutral-700 dark:text-neutral-300" />
            Bills & Subscriptions
          </h1>
          <p className="text-sm text-neutral-500 dark:text-neutral-400 mt-1">
            Overview of active subscriptions, fixed overhead, installment schedules, and automated transfers.
          </p>
        </div>
        <button
          onClick={() => {
            setEditingBill(null);
            setFormError(null);
            setIsModalOpen(true);
          }}
          className="h-9 inline-flex items-center gap-2 px-3.5 text-xs font-medium text-white bg-neutral-900 hover:bg-neutral-800 dark:bg-white dark:text-neutral-900 dark:hover:bg-neutral-100 rounded-md shadow-xs transition-colors cursor-pointer"
        >
          <Plus className="w-4 h-4" />
          Add Item
        </button>
      </div>

      {/* Neutral Stat Strip */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
        <div className="p-4 bg-neutral-50 dark:bg-neutral-950 rounded-lg border border-neutral-200 dark:border-neutral-800 space-y-1">
          <div className="text-xs font-medium text-neutral-500 dark:text-neutral-400 font-mono">
            Monthly Commitment
          </div>
          <div className="mt-1 text-2xl font-bold font-mono tracking-tight text-neutral-900 dark:text-white">
            {displayCurrency} {totalMonthlyCommitment.toFixed(2)}
          </div>
          <p className="mt-1 text-[11px] text-neutral-500 font-mono">Normalized monthly cost</p>
        </div>

        <div className="p-4 bg-neutral-50 dark:bg-neutral-950 rounded-lg border border-neutral-200 dark:border-neutral-800 space-y-1">
          <div className="text-xs font-medium text-neutral-500 dark:text-neutral-400 font-mono">
            Active Subscriptions
          </div>
          <div className="mt-1 text-2xl font-bold font-mono tracking-tight text-neutral-900 dark:text-white">
            {subscriptionsCount}
          </div>
          <p className="mt-1 text-[11px] text-neutral-500 font-mono">SaaS & recurring services</p>
        </div>

        <div className="p-4 bg-neutral-50 dark:bg-neutral-950 rounded-lg border border-neutral-200 dark:border-neutral-800 space-y-1">
          <div className="text-xs font-medium text-neutral-500 dark:text-neutral-400 font-mono">
            Fixed Bills & Transfers
          </div>
          <div className="mt-1 text-2xl font-bold font-mono tracking-tight text-neutral-900 dark:text-white">
            {fixedBillsCount + transfersCount}
          </div>
          <p className="mt-1 text-[11px] text-neutral-500 font-mono">Rent, utilities & auto-savings</p>
        </div>

        <div className="p-4 bg-neutral-50 dark:bg-neutral-950 rounded-lg border border-neutral-200 dark:border-neutral-800 space-y-1">
          <div className="text-xs font-medium text-neutral-500 dark:text-neutral-400 font-mono">
            Installment Plans
          </div>
          <div className="mt-1 text-2xl font-bold font-mono tracking-tight text-neutral-900 dark:text-white">
            {installmentsCount}
          </div>
          <p className="mt-1 text-[11px] text-neutral-500 font-mono">Active loan / BNPL schedules</p>
        </div>
      </div>

      {/* Navigation Tabs */}
      <div className="flex items-center gap-1 border-b border-neutral-200 dark:border-neutral-800 overflow-x-auto">
        {[
          { id: 'all', label: 'All', count: bills.length },
          { id: 'subscription', label: 'Subscriptions', count: subscriptionsCount },
          { id: 'fixed_bill', label: 'Fixed Bills', count: fixedBillsCount },
          { id: 'installment', label: 'Installment Payments', count: installmentsCount },
          { id: 'recurring_transfer', label: 'Recurring Transfers', count: transfersCount },
        ].map((tab) => (
          <button
            key={tab.id}
            onClick={() => setActiveTab(tab.id as 'all' | BillCategory)}
            className={`flex items-center gap-2 px-3 py-2 text-sm font-medium border-b-2 whitespace-nowrap transition-colors cursor-pointer ${
              activeTab === tab.id
                ? 'border-neutral-900 text-neutral-900 dark:border-white dark:text-white font-semibold'
                : 'border-transparent text-neutral-500 hover:text-neutral-700 dark:text-neutral-400 dark:hover:text-neutral-200'
            }`}
          >
            <span>{tab.label}</span>
            <span className="px-1.5 py-0.2 text-xs rounded bg-neutral-100 dark:bg-neutral-800 font-mono text-neutral-600 dark:text-neutral-300">
              {tab.count}
            </span>
          </button>
        ))}
      </div>

      {/* Ledger Table */}
      {isLoading ? (
        <div className="py-12 text-center text-sm font-mono text-neutral-400">Loading recurring bills...</div>
      ) : filteredBills.length === 0 ? (
        <div className="py-16 text-center border border-dashed border-neutral-200 dark:border-neutral-800 rounded-lg">
          <RefreshCcw className="w-8 h-8 mx-auto text-neutral-300 dark:text-neutral-700 mb-2" />
          <p className="text-sm font-medium text-neutral-700 dark:text-neutral-300">No recurring items recorded</p>
          <p className="text-xs text-neutral-400 mt-1">Click "Add Item" to record a new subscription, bill, or installment.</p>
        </div>
      ) : (
        <div className="overflow-x-auto border border-neutral-200 dark:border-neutral-800 rounded-lg bg-white dark:bg-neutral-950">
          <table className="w-full text-left text-sm border-collapse">
            <thead>
              <tr className="border-b border-neutral-200 dark:border-neutral-800 bg-neutral-50 dark:bg-neutral-900/60 text-xs font-semibold uppercase tracking-wider text-neutral-500 dark:text-neutral-400 font-mono">
                <th className="py-3 px-4">Item & Category</th>
                <th className="py-3 px-4">Frequency</th>
                <th className="py-3 px-4">Next Due Date</th>
                <th className="py-3 px-4">Installment Progress</th>
                <th className="py-3 px-4 text-right font-mono">Amount</th>
                <th className="py-3 px-4 text-right">Actions</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-neutral-200 dark:divide-neutral-800">
              {filteredBills.map((bill) => {
                const badge = getCategoryBadge(bill.category);
                const BadgeIcon = badge.icon;
                const formattedAmount = (bill.amount_minor / 100).toFixed(2);
                const isOverdue = new Date(bill.next_due_at) < new Date();

                return (
                  <tr
                    key={bill.id}
                    className="hover:bg-neutral-50 dark:hover:bg-neutral-900/40 transition-colors"
                  >
                    <td className="py-3.5 px-4">
                      <div className="flex items-center gap-3">
                        <div className="p-2 rounded bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-300">
                          <BadgeIcon className="w-4 h-4" />
                        </div>
                        <div>
                          <div className="flex items-center gap-2">
                            <span className="font-medium text-neutral-900 dark:text-white">
                              {bill.name}
                            </span>
                            {bill.auto_pay && (
                              <span className="inline-flex items-center gap-0.5 px-1.5 py-0.2 text-xs font-mono bg-emerald-50 text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300 rounded border border-emerald-200 dark:border-emerald-800">
                                <Zap className="w-3 h-3" /> Auto
                              </span>
                            )}
                            {bill.remind_before_minutes ? (
                              <span
                                className="inline-flex items-center gap-0.5 px-1.5 py-0.2 text-xs font-mono bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400 rounded border border-neutral-200 dark:border-neutral-700"
                                title={`Reminder ${leadLabel(bill.remind_before_minutes)}`}
                              >
                                <BellRing className="w-3 h-3" /> {leadLabel(bill.remind_before_minutes)}
                              </span>
                            ) : null}
                          </div>
                          <div className="flex items-center gap-2 mt-0.5">
                            <span className="text-xs text-neutral-500 dark:text-neutral-400 font-mono">
                              {badge.label}
                            </span>
                            {bill.notes && (
                              <span className="text-xs text-neutral-400 truncate max-w-xs">
                                • {bill.notes}
                              </span>
                            )}
                          </div>
                        </div>
                      </div>
                    </td>

                    <td className="py-3.5 px-4 capitalize text-neutral-600 dark:text-neutral-300 font-mono text-xs">
                      {bill.frequency}
                    </td>

                    <td className="py-3.5 px-4 font-mono text-xs">
                      <span className={isOverdue ? 'text-rose-600 font-medium' : 'text-neutral-600 dark:text-neutral-300'}>
                        {bill.next_due_at.slice(0, 10)}
                      </span>
                    </td>

                    <td className="py-3.5 px-4">
                      {bill.category === 'installment' && bill.total_installments ? (
                        <div className="w-36 space-y-1">
                          <div className="flex justify-between text-xs font-mono text-neutral-500">
                            <span>{bill.paid_installments} / {bill.total_installments}</span>
                            <span>{Math.round((bill.paid_installments / bill.total_installments) * 100)}%</span>
                          </div>
                          <div className="w-full h-1.5 bg-neutral-100 dark:bg-neutral-800 rounded-full overflow-hidden">
                            <div
                              className="h-full bg-neutral-900 dark:bg-white rounded-full transition-all"
                              style={{
                                width: `${Math.min(
                                  100,
                                  (bill.paid_installments / bill.total_installments) * 100
                                )}%`,
                              }}
                            />
                          </div>
                        </div>
                      ) : (
                        <span className="text-xs text-neutral-400 font-mono">—</span>
                      )}
                    </td>

                    <td className="py-3.5 px-4 text-right font-mono font-medium text-neutral-900 dark:text-white">
                      {bill.currency} {formattedAmount}
                    </td>

                    <td className="py-3.5 px-4 text-right">
                      <div className="flex items-center justify-end gap-1.5">
                        <button
                          onClick={() => payMutation.mutate(bill.id)}
                          disabled={payMutation.isPending}
                          title="Mark Paid for current cycle"
                          className="inline-flex items-center gap-1 px-2.5 py-1 text-xs font-medium text-neutral-700 dark:text-neutral-200 bg-neutral-100 hover:bg-neutral-200 dark:bg-neutral-800 dark:hover:bg-neutral-700 rounded transition-colors cursor-pointer"
                        >
                          <CheckCircle2 className="w-3.5 h-3.5 text-emerald-500" />
                          Pay
                        </button>
                        <button
                          onClick={() => {
                            setEditingBill(bill);
                            setFormError(null);
                            setIsModalOpen(true);
                          }}
                          className="p-1 text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-200 rounded transition-colors cursor-pointer"
                        >
                          <Pencil className="w-3.5 h-3.5" />
                        </button>
                        <button
                          onClick={() => deleteMutation.mutate(bill.id)}
                          className="p-1 text-neutral-400 hover:text-rose-600 dark:hover:text-rose-400 rounded transition-colors cursor-pointer"
                        >
                          <Trash2 className="w-3.5 h-3.5" />
                        </button>
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}

      {/* Modal */}
      <BillModal
        isOpen={isModalOpen}
        onClose={() => {
          setIsModalOpen(false);
          setEditingBill(null);
          setFormError(null);
        }}
        onSubmit={handleModalSubmit}
        initialData={editingBill}
        isSubmitting={createMutation.isPending || updateMutation.isPending}
        errorMsg={formError}
      />
    </div>
  );
};

/** Short form of a lead time, matching the modal's option wording. */
function leadLabel(minutes: number): string {
  if (minutes % 10080 === 0) return `${minutes / 10080}w before`;
  if (minutes % 1440 === 0) return `${minutes / 1440}d before`;
  if (minutes % 60 === 0) return `${minutes / 60}h before`;
  return `${minutes}m before`;
}

export const recurringBillsModule: FrontendModule = {
  id: 'recurring_bills',
  name: 'Recurring Bills',
  description: 'Manage subscriptions, fixed bills, installments, and transfers',
  iconName: 'RefreshCcw',
  navTab: {
    label: 'Bills & Subscriptions',
  },
  MainView: RecurringBillsView,
  handleSseEvent: (event, queryClient) => {
    if (
      event.type === 'RecurringBillCreated' ||
      event.type === 'RecurringBillUpdated' ||
      event.type === 'RecurringBillDeleted' ||
      event.type === 'RecurringBillPaid'
    ) {
      queryClient.invalidateQueries({ queryKey: QUERY_KEY });
    }
  },
};
