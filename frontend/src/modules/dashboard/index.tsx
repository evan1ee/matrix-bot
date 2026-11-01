import React from 'react';
import { useQuery } from '@tanstack/react-query';
import { CheckSquare, Clock, TrendingUp, Receipt, Plus } from 'lucide-react';
import { FrontendModule } from '../../core/types';
import { fetchApi } from '../../core/apiClient';
import { BriefingCard } from './BriefingCard';

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
    data-testid="stat-card"
    data-stat={label}
    className="text-left bg-[var(--color-surface-card)] border border-[var(--color-border-subtle)] rounded-lg p-4 flex flex-col justify-between cursor-pointer transition-colors duration-150 hover:border-[var(--color-border-default)] hover:bg-[var(--color-surface-hover)]"
  >
    <div className="flex items-center justify-between text-xs font-medium text-[var(--color-text-secondary)]">
      <span>{label}</span>
      <Icon className="text-[var(--color-text-tertiary)] w-3.5 h-3.5" />
    </div>
    <div className="my-2 text-2xl sm:text-3xl font-semibold font-sans tracking-tight tabular-nums text-[var(--color-text-primary)]">
      {value}
    </div>
    <div className="text-[11px] text-[var(--color-text-tertiary)] font-mono">
      {subtitle}
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
  const { data: todosData, isLoading: todosLoading } = useQuery<TodoListResponse>({
    queryKey: ['todos', 'dashboard'],
    queryFn: () => fetchApi('/todos?completed=false&limit=1'),
  });

  const { data: remindersData, isLoading: remindersLoading } = useQuery<ReminderListResponse>({
    queryKey: ['reminders', 'dashboard'],
    queryFn: () => fetchApi('/reminders?limit=1'),
  });

  const { data: portfolioData, isLoading: portfolioLoading } = useQuery<PortfolioCurrencySummary[]>({
    queryKey: ['portfolio-summary', 'dashboard'],
    queryFn: () => fetchApi('/portfolio/summary'),
  });

  const { data: billsData, isLoading: billsLoading } = useQuery<RecurringBill[]>({
    queryKey: ['recurring-bills', 'dashboard'],
    queryFn: () => fetchApi('/recurring-bills'),
  });

  // `total` counts every row matching the query, not just the page fetched, so
  // these tiles stay correct past the first page. Both queries ask for one row
  // purely to reach that count.
  const activeTasks = todosLoading ? '—' : (todosData?.total ?? 0);
  const activeReminders = remindersLoading ? '—' : (remindersData?.total ?? 0);
  const currenciesTracked = portfolioLoading ? '—' : (portfolioData?.length ?? 0);
  const activeBills = billsLoading ? '—' : (billsData?.filter((bill) => bill.status === 'active')?.length ?? 0);

  return (
    <div className="space-y-6">
      {/* Welcome Header */}
      <div>
        <h2 className="text-xl font-semibold tracking-tight text-[var(--color-text-primary)]">Welcome back</h2>
        <p className="text-xs text-[var(--color-text-secondary)] mt-1">{currentDate}</p>
      </div>

      {/* Quick Stats Grid */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
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

      <BriefingCard />

      {/* Quick Actions */}
      <div className="flex flex-wrap gap-2.5 pt-2">
        <button
          onClick={() => onNavigate?.('todo')}
          className="flex items-center gap-2 px-3.5 py-2 rounded-md border border-[var(--color-border-default)] text-xs font-medium text-[var(--color-text-secondary)] hover:bg-[var(--color-surface-hover)] hover:text-[var(--color-text-primary)] transition-colors duration-150 cursor-pointer shadow-2xs"
        >
          <Plus className="w-3.5 h-3.5" />
          Add Task
        </button>
        <button
          onClick={() => onNavigate?.('reminder')}
          className="flex items-center gap-2 px-3.5 py-2 rounded-md border border-[var(--color-border-default)] text-xs font-medium text-[var(--color-text-secondary)] hover:bg-[var(--color-surface-hover)] hover:text-[var(--color-text-primary)] transition-colors duration-150 cursor-pointer shadow-2xs"
        >
          <Plus className="w-3.5 h-3.5" />
          Set Reminder
        </button>
        <button
          onClick={() => onNavigate?.('finance')}
          className="flex items-center gap-2 px-3.5 py-2 rounded-md border border-[var(--color-border-default)] text-xs font-medium text-[var(--color-text-secondary)] hover:bg-[var(--color-surface-hover)] hover:text-[var(--color-text-primary)] transition-colors duration-150 cursor-pointer shadow-2xs"
        >
          <Plus className="w-3.5 h-3.5" />
          Log Expense
        </button>
      </div>

      {/* System Status Footer */}
      <div className="pt-6 pb-2 text-[11px] font-mono text-[var(--color-text-tertiary)] border-t border-[var(--color-border-subtle)] flex flex-col sm:flex-row sm:justify-between gap-2">
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
