import React from 'react';
import { useQuery } from '@tanstack/react-query';
import { CheckSquare, Clock, TrendingUp, Receipt, Plus } from 'lucide-react';
import { FrontendModule } from '../../core/types';
import { fetchApi } from '../../core/apiClient';

interface TodoListResponse {
  items: Array<{ id: number }>;
  page: number;
  limit: number;
  total: number;
}

interface ReminderListResponse {
  items: Array<{ id: number }>;
  page: number;
  limit: number;
  total: number;
}

interface PortfolioCurrencySummary {
  currency: string;
}

interface RecurringBill {
  id: number;
  status: string;
}

const StatCard = ({
  label,
  value,
  subtitle,
  icon: Icon,
  onClick,
}: {
  label: string;
  value: string | number;
  subtitle: string;
  icon: React.ElementType;
  onClick?: () => void;
}) => (
  <button
    type="button"
    onClick={onClick}
    // A stable hook for tests, matching `todo-row` and `datetime-picker`
    // elsewhere. The card is a <button>, so tests that walked up to an ancestor
    // <div> by class name silently matched nothing.
    data-testid="stat-card"
    data-stat={label}
    className="text-left bg-[var(--color-surface-card)] border border-[var(--color-border-subtle)] rounded-lg p-5 flex flex-col justify-between cursor-pointer transition duration-150 hover:border-[var(--color-border-default)] hover:bg-[var(--color-surface-hover)]"
  >
    <div className="flex justify-between items-start mb-4">
      <div className="text-3xl font-bold font-mono tracking-tight text-[var(--color-text-primary)]">
        {value}
      </div>
      <Icon className="text-[var(--color-text-tertiary)] w-4 h-4" />
    </div>
    <div>
      <div className="text-xs font-mono uppercase text-[var(--color-text-secondary)]">
        {label}
      </div>
      <div className="text-[11px] text-[var(--color-text-tertiary)] font-mono mt-1">
        {subtitle}
      </div>
    </div>
  </button>
);

const DashboardView = ({ onNavigate }: { onNavigate?: (moduleId: string) => void }) => {
  const currentDate = new Date().toLocaleDateString('en-US', {
    weekday: 'long',
    year: 'numeric',
    month: 'long',
    day: 'numeric',
  });

  // Each key must share its root with the key the owning module invalidates,
  // or React Query's prefix matching never reaches these cards and the tile
  // silently shows a stale figure while the module page shows the live one.
  const { data: todosData } = useQuery<TodoListResponse>({
    queryKey: ['todos', 'dashboard'],
    queryFn: () => fetchApi('/todos?completed=false&limit=1'),
  });

  const { data: remindersData } = useQuery<ReminderListResponse>({
    queryKey: ['reminders', 'dashboard'],
    queryFn: () => fetchApi('/reminders?limit=1'),
  });

  const { data: portfolioData } = useQuery<PortfolioCurrencySummary[]>({
    queryKey: ['portfolio-summary', 'dashboard'],
    queryFn: () => fetchApi('/portfolio/summary'),
  });

  const { data: billsData } = useQuery<RecurringBill[]>({
    queryKey: ['recurring-bills', 'dashboard'],
    queryFn: () => fetchApi('/recurring-bills'),
  });

  // `total` counts every row matching the query, not just the page fetched, so
  // these tiles stay correct past the first page. Both queries ask for one row
  // purely to reach that count.
  const activeTasks = todosData?.total ?? 0;
  const activeReminders = remindersData?.total ?? 0;
  const currenciesTracked = portfolioData?.length ?? 0;
  const activeBills = billsData?.filter((bill) => bill.status === 'active')?.length ?? 0;

  return (
    <div className="flex flex-col min-h-full max-w-5xl w-full mx-auto">
      {/* Main Content */}
      <div className="flex-1 space-y-8 pb-8">
        {/* Welcome Header */}
        <div>
          <h1 className="text-2xl font-semibold text-[var(--color-text-primary)]">Welcome back</h1>
          <p className="text-[var(--color-text-secondary)] mt-1">{currentDate}</p>
        </div>

        {/* Quick Stats Grid */}
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
          <StatCard
            label="Active Tasks"
            value={activeTasks}
            subtitle="via Matrix & API"
            icon={CheckSquare}
            onClick={() => onNavigate?.('todo')}
          />
          <StatCard
            label="Reminders"
            value={activeReminders}
            subtitle="pending execution"
            icon={Clock}
            onClick={() => onNavigate?.('reminder')}
          />
          <StatCard
            label="Currencies Tracked"
            value={currenciesTracked}
            subtitle="portfolio positions"
            icon={TrendingUp}
            onClick={() => onNavigate?.('portfolio')}
          />
          <StatCard
            label="Active Bills"
            value={activeBills}
            subtitle="recurring charges"
            icon={Receipt}
            onClick={() => onNavigate?.('recurring_bills')}
          />
        </div>

        {/* Quick Actions */}
        <div className="flex flex-wrap gap-3">
          <button
            onClick={() => onNavigate?.('todo')}
            className="flex items-center gap-2 px-4 py-2 min-h-11 rounded-full border border-[var(--color-border-default)] text-sm font-medium text-[var(--color-text-secondary)] hover:bg-[var(--color-surface-hover)] hover:text-[var(--color-text-primary)] transition duration-150 cursor-pointer"
          >
            <Plus className="w-3.5 h-3.5" />
            Add Task
          </button>
          <button
            onClick={() => onNavigate?.('reminder')}
            className="flex items-center gap-2 px-4 py-2 min-h-11 rounded-full border border-[var(--color-border-default)] text-sm font-medium text-[var(--color-text-secondary)] hover:bg-[var(--color-surface-hover)] hover:text-[var(--color-text-primary)] transition duration-150 cursor-pointer"
          >
            <Plus className="w-3.5 h-3.5" />
            Set Reminder
          </button>
          <button
            onClick={() => onNavigate?.('finance')}
            className="flex items-center gap-2 px-4 py-2 min-h-11 rounded-full border border-[var(--color-border-default)] text-sm font-medium text-[var(--color-text-secondary)] hover:bg-[var(--color-surface-hover)] hover:text-[var(--color-text-primary)] transition duration-150 cursor-pointer"
          >
            <Plus className="w-3.5 h-3.5" />
            Log Expense
          </button>
        </div>
      </div>

      {/* System Status Footer - Pinned cleanly to the bottom */}
      <div className="mt-auto pt-6 pb-2 text-[11px] font-mono text-[var(--color-text-tertiary)] border-t border-[var(--color-border-subtle)] flex flex-col sm:flex-row sm:justify-between gap-2">
        <p>Powered by Matrix SDK + Axum REST API</p>
        <p>Real-time sync via Server-Sent Events</p>
      </div>
    </div>
  );
};

export const dashboardModule: FrontendModule = {
  id: 'dashboard',
  name: 'Dashboard',
  description: 'Overview of your workspace',
  navTab: { label: 'Dashboard' },
  MainView: DashboardView,
};
