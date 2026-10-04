import React, { useState } from 'react';
import {
  AlertCircle,
  CheckCircle,
  Clock,
  Eye,
  Loader2,
  Play,
  RotateCw,
  Search,
  Sparkles,
  XCircle,
} from 'lucide-react';
import { useAgentRuns, useCancelRun } from '../api';
import { AgentRun, AgentRunStatus } from '../types';
import { RunDetailDrawer } from './RunDetailDrawer';
import { StartRunModal } from './StartRunModal';
import { formatDateTimeInZone, useSettings } from '../../../core/settings';

export const RunList: React.FC = () => {
  const { data: runs = [], isLoading, isError, refetch } = useAgentRuns();
  const cancelMutation = useCancelRun();
  const { timezone } = useSettings();

  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [isStartModalOpen, setIsStartModalOpen] = useState(false);
  const [statusFilter, setStatusFilter] = useState<AgentRunStatus | 'all'>('all');
  const [searchQuery, setSearchQuery] = useState('');

  const activeCount = runs.filter(
    (r) => r.status === 'running' || r.status === 'planning' || r.status === 'waiting_approval'
  ).length;
  const waitingApprovalCount = runs.filter((r) => r.status === 'waiting_approval').length;
  const totalTokens = runs.reduce((acc, r) => acc + (r.total_tokens || 0), 0);

  const filteredRuns = runs.filter((r) => {
    if (statusFilter !== 'all' && r.status !== statusFilter) return false;
    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase();
      return r.goal.toLowerCase().includes(q) || r.task_name.toLowerCase().includes(q) || r.id.toLowerCase().includes(q);
    }
    return true;
  });

  const getStatusBadge = (status: string) => {
    switch (status) {
      case 'completed':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium bg-emerald-100 dark:bg-emerald-950/60 text-emerald-800 dark:text-emerald-300">
            <CheckCircle size={11} />
            Completed
          </span>
        );
      case 'waiting_approval':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium bg-amber-100 dark:bg-amber-950/60 text-amber-800 dark:text-amber-300 animate-pulse">
            <AlertCircle size={11} />
            Approval Needed
          </span>
        );
      case 'running':
      case 'planning':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium bg-blue-100 dark:bg-blue-950/60 text-blue-800 dark:text-blue-300">
            <Loader2 size={11} className="animate-spin" />
            {status === 'planning' ? 'Planning' : 'Running'}
          </span>
        );
      case 'failed':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium bg-red-100 dark:bg-red-950/60 text-red-800 dark:text-red-300">
            <XCircle size={11} />
            Failed
          </span>
        );
      case 'cancelled':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400">
            Cancelled
          </span>
        );
      default:
        return (
          <span className="px-2 py-0.5 rounded-full text-[10px] font-medium bg-neutral-100 dark:bg-neutral-800 text-neutral-700 dark:text-neutral-300">
            {status}
          </span>
        );
    }
  };

  return (
    <div className="space-y-4">
      {/* Top Controls Bar */}
      <div className="flex flex-col sm:flex-row items-stretch sm:items-center justify-between gap-3">
        <div className="flex items-center gap-3">
          <button
            type="button"
            onClick={() => setIsStartModalOpen(true)}
            className="px-3.5 py-2 rounded-lg bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 font-medium text-xs inline-flex items-center gap-1.5 hover:bg-neutral-800 dark:hover:bg-neutral-200 transition-colors shadow-xs cursor-pointer"
          >
            <Sparkles size={14} />
            Start Run...
          </button>
          <button
            type="button"
            onClick={() => refetch()}
            className="p-2 rounded-lg border border-neutral-200 dark:border-neutral-800 text-neutral-600 dark:text-neutral-400 hover:bg-neutral-50 dark:hover:bg-neutral-850 transition-colors"
            title="Refresh runs"
          >
            <RotateCw size={14} />
          </button>
        </div>

        {/* Stats Strip */}
        <div className="flex items-center gap-4 text-xs font-sans text-neutral-500">
          <div>
            Active: <span className="font-semibold text-neutral-900 dark:text-neutral-100 font-mono">{activeCount}</span>
          </div>
          {waitingApprovalCount > 0 && (
            <div className="text-amber-600 dark:text-amber-400 font-medium">
              Approvals: <span className="font-mono font-bold">{waitingApprovalCount}</span>
            </div>
          )}
          <div>
            Tokens: <span className="font-semibold text-neutral-900 dark:text-neutral-100 font-mono">{totalTokens.toLocaleString()}</span>
          </div>
        </div>
      </div>

      {/* Filter Tabs & Search */}
      <div className="flex flex-col sm:flex-row items-stretch sm:items-center justify-between gap-3 pt-2">
        <div className="flex items-center gap-1 overflow-x-auto pb-1 sm:pb-0 text-xs">
          {(
            [
              ['all', 'All'],
              ['running', 'Active'],
              ['waiting_approval', 'Approvals'],
              ['completed', 'Completed'],
              ['failed', 'Failed'],
            ] as const
          ).map(([val, label]) => (
            <button
              key={val}
              type="button"
              onClick={() => setStatusFilter(val)}
              className={`px-2.5 py-1 rounded-md transition-colors ${
                statusFilter === val
                  ? 'bg-neutral-200 dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100 font-medium'
                  : 'text-neutral-600 dark:text-neutral-400 hover:bg-neutral-100 dark:hover:bg-neutral-850'
              }`}
            >
              {label}
            </button>
          ))}
        </div>

        <div className="relative">
          <Search size={13} className="absolute left-2.5 top-2.5 text-neutral-400" />
          <input
            type="text"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder="Search goals or tasks..."
            className="w-full sm:w-60 pl-8 pr-3 py-1.5 rounded-md bg-white dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 text-xs focus:outline-none focus:ring-1 focus:ring-neutral-400"
          />
        </div>
      </div>

      {/* Runs Table */}
      {isLoading ? (
        <div className="flex flex-col items-center justify-center py-20 text-neutral-400 gap-2">
          <Loader2 className="w-6 h-6 animate-spin" />
          <span className="text-xs">Loading agent runs...</span>
        </div>
      ) : isError ? (
        <div className="p-4 rounded-lg bg-red-50 dark:bg-red-950/40 border border-red-200 dark:border-red-900 text-red-600 dark:text-red-400 text-xs">
          Failed to load agent runs.
        </div>
      ) : filteredRuns.length === 0 ? (
        <div className="py-16 text-center text-neutral-400 border border-dashed border-neutral-200 dark:border-neutral-800 rounded-xl text-xs space-y-2">
          <p>No agent runs match the selected filter.</p>
          <button
            type="button"
            onClick={() => setIsStartModalOpen(true)}
            className="text-neutral-900 dark:text-neutral-100 font-medium hover:underline cursor-pointer"
          >
            Launch a new goal &rarr;
          </button>
        </div>
      ) : (
        <div className="rounded-xl border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 overflow-hidden shadow-xs">
          <div className="overflow-x-auto">
            <table className="w-full text-left text-xs">
              <thead className="bg-neutral-50 dark:bg-neutral-950/80 border-b border-neutral-200 dark:border-neutral-800 text-[11px] font-medium text-neutral-500 uppercase tracking-wider">
                <tr>
                  <th className="py-3 px-4">Status</th>
                  <th className="py-3 px-4">Goal / Objective</th>
                  <th className="py-3 px-4">Steps</th>
                  <th className="py-3 px-4">Tokens</th>
                  <th className="py-3 px-4">Started</th>
                  <th className="py-3 px-4 text-right">Actions</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-neutral-100 dark:divide-neutral-800">
                {filteredRuns.map((run) => {
                  const percent = Math.min(100, Math.round((run.current_step / Math.max(1, run.max_steps)) * 100));
                  const isCancellable =
                    run.status === 'running' || run.status === 'planning' || run.status === 'waiting_approval';

                  return (
                    <tr
                      key={run.id}
                      className="hover:bg-neutral-50/60 dark:hover:bg-neutral-850/40 transition-colors group cursor-pointer"
                      onClick={() => setSelectedRunId(run.id)}
                    >
                      <td className="py-3.5 px-4 whitespace-nowrap">
                        {getStatusBadge(run.status)}
                      </td>
                      <td className="py-3.5 px-4 max-w-xs sm:max-w-md">
                        <div className="font-medium text-neutral-900 dark:text-neutral-100 truncate">
                          {run.goal}
                        </div>
                        <div className="font-mono text-[10px] text-neutral-400 truncate mt-0.5">
                          {run.task_name} &bull; {run.id}
                        </div>
                      </td>
                      <td className="py-3.5 px-4 whitespace-nowrap">
                        <div className="flex items-center gap-2">
                          <span className="font-mono text-[11px] text-neutral-700 dark:text-neutral-300">
                            {run.current_step}/{run.max_steps}
                          </span>
                          <div className="w-16 h-1.5 rounded-full bg-neutral-200 dark:bg-neutral-800 overflow-hidden">
                            <div
                              className="h-full bg-neutral-900 dark:bg-neutral-100 transition-all duration-300"
                              style={{ width: `${percent}%` }}
                            />
                          </div>
                        </div>
                      </td>
                      <td className="py-3.5 px-4 whitespace-nowrap font-mono text-neutral-600 dark:text-neutral-400">
                        {run.total_tokens.toLocaleString()}
                      </td>
                      <td className="py-3.5 px-4 whitespace-nowrap font-mono text-[11px] text-neutral-500">
                        {formatDateTimeInZone(run.started_at, timezone)}
                      </td>
                      <td className="py-3.5 px-4 text-right whitespace-nowrap" onClick={(e) => e.stopPropagation()}>
                        <div className="flex items-center justify-end gap-1.5">
                          <button
                            type="button"
                            onClick={() => setSelectedRunId(run.id)}
                            className="p-1 rounded text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition-colors"
                            title="Inspect Run"
                          >
                            <Eye size={14} />
                          </button>
                          {isCancellable && (
                            <button
                              type="button"
                              disabled={cancelMutation.isPending}
                              onClick={() => cancelMutation.mutate(run.id)}
                              className="px-2 py-1 rounded bg-red-50 hover:bg-red-100 dark:bg-red-950/40 dark:hover:bg-red-900/60 text-red-600 dark:text-red-400 text-[10px] font-medium transition-colors"
                            >
                              Cancel
                            </button>
                          )}
                        </div>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </div>
      )}

      {/* Start Run Modal */}
      <StartRunModal
        isOpen={isStartModalOpen}
        onClose={() => setIsStartModalOpen(false)}
        onSuccess={(id) => setSelectedRunId(id)}
      />

      {/* Run Detail Drawer */}
      <RunDetailDrawer
        runId={selectedRunId}
        onClose={() => setSelectedRunId(null)}
      />
    </div>
  );
};
