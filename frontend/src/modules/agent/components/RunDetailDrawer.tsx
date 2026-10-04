import React, { useState } from 'react';
import {
  AlertCircle,
  CheckCircle,
  ChevronDown,
  ChevronRight,
  Clock,
  Code2,
  Copy,
  Cpu,
  Loader2,
  X,
  XCircle,
} from 'lucide-react';
import { useAgentRunDetail, useCancelRun } from '../api';
import { formatDateTimeInZone, useSettings } from '../../../core/settings';
import { copyToClipboard } from '../../../core/clipboard';

interface RunDetailDrawerProps {
  runId: string | null;
  onClose: () => void;
}

export const RunDetailDrawer: React.FC<RunDetailDrawerProps> = ({ runId, onClose }) => {
  const { data: detail, isLoading, isError } = useAgentRunDetail(runId);
  const cancelMutation = useCancelRun();
  const { timezone } = useSettings();

  const [expandedThoughts, setExpandedThoughts] = useState<Record<number, boolean>>({});
  const [copiedStep, setCopiedStep] = useState<number | null>(null);

  if (!runId) return null;

  const run = detail?.run;
  const steps = detail?.steps || [];

  const toggleThought = (stepNum: number) => {
    setExpandedThoughts((prev) => {
      const current = prev[stepNum] ?? true;
      return {
        ...prev,
        [stepNum]: !current,
      };
    });
  };

  const handleCopy = (stepNum: number, content: string) => {
    copyToClipboard(content);
    setCopiedStep(stepNum);
    setTimeout(() => setCopiedStep(null), 1500);
  };

  const isCancellable =
    run?.status === 'running' || run?.status === 'planning' || run?.status === 'waiting_approval';

  const getStatusBadge = (status: string) => {
    switch (status) {
      case 'completed':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[11px] font-medium bg-emerald-100 dark:bg-emerald-950/60 text-emerald-800 dark:text-emerald-300">
            <CheckCircle size={12} />
            Completed
          </span>
        );
      case 'waiting_approval':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[11px] font-medium bg-amber-100 dark:bg-amber-950/60 text-amber-800 dark:text-amber-300">
            <AlertCircle size={12} />
            Waiting Approval
          </span>
        );
      case 'running':
      case 'planning':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[11px] font-medium bg-blue-100 dark:bg-blue-950/60 text-blue-800 dark:text-blue-300">
            <Loader2 size={12} className="animate-spin" />
            {status === 'planning' ? 'Planning' : 'Executing'}
          </span>
        );
      case 'failed':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[11px] font-medium bg-red-100 dark:bg-red-950/60 text-red-800 dark:text-red-300">
            <XCircle size={12} />
            Failed
          </span>
        );
      case 'cancelled':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[11px] font-medium bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400">
            Cancelled
          </span>
        );
      default:
        return (
          <span className="px-2 py-0.5 rounded-full text-[11px] font-medium bg-neutral-100 dark:bg-neutral-800 text-neutral-700 dark:text-neutral-300">
            {status}
          </span>
        );
    }
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      className="fixed inset-0 z-50 flex justify-end bg-black/40 backdrop-blur-xs animate-in fade-in duration-150"
      onKeyDown={(e) => {
        if (e.key === 'Escape') onClose();
      }}
    >
      <div className="relative w-full max-w-2xl bg-white dark:bg-neutral-900 border-l border-neutral-200 dark:border-neutral-800 shadow-2xl flex flex-col h-full overflow-hidden text-xs font-sans text-neutral-900 dark:text-neutral-100 animate-in slide-in-from-right duration-200">
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-neutral-200 dark:border-neutral-800 shrink-0">
          <div className="flex items-center gap-2.5">
            <Cpu className="w-5 h-5 text-neutral-700 dark:text-neutral-300" />
            <div>
              <h2 className="text-sm font-semibold">Run Details</h2>
              <span className="font-mono text-[11px] text-neutral-400">
                {runId}
              </span>
            </div>
          </div>
          <div className="flex items-center gap-2">
            {isCancellable && (
              <button
                type="button"
                disabled={cancelMutation.isPending}
                onClick={() => cancelMutation.mutate(runId)}
                className="px-2.5 py-1.5 rounded bg-red-50 hover:bg-red-100 dark:bg-red-950/40 dark:hover:bg-red-900/60 text-red-600 dark:text-red-400 border border-red-200 dark:border-red-800 font-medium transition-colors"
              >
                {cancelMutation.isPending ? 'Cancelling...' : 'Cancel Run'}
              </button>
            )}
            <button
              type="button"
              onClick={onClose}
              className="text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-200 p-1.5 rounded transition-colors"
            >
              <X size={18} />
            </button>
          </div>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto p-6 space-y-6">
          {isLoading ? (
            <div className="flex flex-col items-center justify-center py-20 text-neutral-400 gap-2">
              <Loader2 className="w-6 h-6 animate-spin" />
              <span>Loading execution trace...</span>
            </div>
          ) : isError || !run ? (
            <div className="p-4 rounded-lg bg-red-50 dark:bg-red-950/40 border border-red-200 dark:border-red-900 text-red-600 dark:text-red-400">
              Failed to load run details.
            </div>
          ) : (
            <>
              {/* Overview Box */}
              <div className="p-4 rounded-xl bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 space-y-3">
                <div className="flex items-start justify-between gap-3">
                  <div className="space-y-1">
                    <span className="text-[10px] font-semibold tracking-wider uppercase text-neutral-400">
                      Goal Objective
                    </span>
                    <p className="text-xs font-medium text-neutral-900 dark:text-neutral-100 leading-relaxed">
                      {run.goal}
                    </p>
                  </div>
                  <div>{getStatusBadge(run.status)}</div>
                </div>

                <div className="grid grid-cols-2 sm:grid-cols-4 gap-3 pt-3 border-t border-neutral-200 dark:border-neutral-800 text-[11px]">
                  <div>
                    <span className="text-neutral-400 block">Steps Progress</span>
                    <span className="font-mono font-medium">
                      {run.current_step} / {run.max_steps}
                    </span>
                  </div>
                  <div>
                    <span className="text-neutral-400 block">Tokens Spent</span>
                    <span className="font-mono font-medium">
                      {run.total_tokens.toLocaleString()}
                    </span>
                  </div>
                  <div>
                    <span className="text-neutral-400 block">Started</span>
                    <span className="font-mono text-neutral-700 dark:text-neutral-300">
                      {formatDateTimeInZone(run.started_at, timezone)}
                    </span>
                  </div>
                  <div>
                    <span className="text-neutral-400 block">Task Name</span>
                    <span className="font-mono text-neutral-700 dark:text-neutral-300 truncate block">
                      {run.task_name}
                    </span>
                  </div>
                </div>

                {run.error && (
                  <div className="p-2.5 rounded bg-red-50 dark:bg-red-950/40 border border-red-200 dark:border-red-900 text-red-600 dark:text-red-400 text-xs">
                    <span className="font-semibold block mb-0.5">Execution Error:</span>
                    <pre className="font-mono text-[11px] whitespace-pre-wrap">{run.error}</pre>
                  </div>
                )}
              </div>

              {/* Steps Timeline */}
              <div className="space-y-3">
                <h3 className="text-xs font-semibold text-neutral-500 uppercase tracking-wider">
                  Execution Trace ({steps.length} Steps)
                </h3>

                {steps.length === 0 ? (
                  <div className="py-8 text-center text-neutral-400 border border-dashed border-neutral-200 dark:border-neutral-800 rounded-lg">
                    {run.status === 'planning'
                      ? 'Agent is analyzing goal and formulating first step...'
                      : 'No steps recorded for this run.'}
                  </div>
                ) : (
                  <div className="space-y-3">
                    {steps.map((step) => {
                      const isExpanded = expandedThoughts[step.step_number] ?? true;
                      return (
                        <div
                          key={step.id}
                          className="rounded-lg border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900/60 overflow-hidden"
                        >
                          {/* Step Header */}
                          <div className="flex items-center justify-between px-3.5 py-2.5 bg-neutral-50/60 dark:bg-neutral-850/40 border-b border-neutral-100 dark:border-neutral-800">
                            <div className="flex items-center gap-2">
                              <span className="font-mono font-semibold px-2 py-0.5 rounded bg-neutral-200 dark:bg-neutral-800 text-neutral-800 dark:text-neutral-200 text-[11px]">
                                #{step.step_number}
                              </span>
                              {step.tool_name ? (
                                <span className="inline-flex items-center gap-1 font-mono text-neutral-700 dark:text-neutral-300 text-[11px]">
                                  <Code2 size={12} className="text-neutral-400" />
                                  {step.tool_name}
                                </span>
                              ) : (
                                <span className="text-neutral-400 text-[11px]">Synthesizing response</span>
                              )}
                            </div>
                            <div className="flex items-center gap-3 text-[11px] text-neutral-400 font-mono">
                              <span>{step.tokens_used} tokens</span>
                              <span className="capitalize">{step.status}</span>
                            </div>
                          </div>

                          {/* Step Body */}
                          <div className="p-3.5 space-y-3">
                            {/* Thought Reasoning */}
                            {step.thought && (
                              <div>
                                <button
                                  type="button"
                                  onClick={() => toggleThought(step.step_number)}
                                  className="flex items-center gap-1 text-[11px] font-medium text-neutral-500 hover:text-neutral-700 dark:hover:text-neutral-300 mb-1"
                                >
                                  {isExpanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
                                  Thought Reasoning
                                </button>
                                {isExpanded && (
                                  <div className="p-2.5 rounded bg-neutral-50 dark:bg-neutral-950/80 border border-neutral-100 dark:border-neutral-850 text-neutral-700 dark:text-neutral-300 text-[11px] leading-relaxed whitespace-pre-wrap font-sans">
                                    {step.thought}
                                  </div>
                                )}
                              </div>
                            )}

                            {/* Tool Call Input */}
                            {step.tool_input !== null && step.tool_input !== undefined && (
                              <div className="space-y-1">
                                <div className="flex items-center justify-between text-[11px] text-neutral-400 font-medium">
                                  <span>Tool Arguments</span>
                                  <button
                                    type="button"
                                    onClick={() =>
                                      handleCopy(step.step_number, JSON.stringify(step.tool_input, null, 2))
                                    }
                                    className="inline-flex items-center gap-1 text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-200"
                                  >
                                    <Copy size={11} />
                                    {copiedStep === step.step_number ? 'Copied' : 'Copy'}
                                  </button>
                                </div>
                                <pre className="p-2 rounded bg-neutral-950 text-neutral-200 text-[10px] font-mono overflow-x-auto max-h-36">
                                  {JSON.stringify(step.tool_input, null, 2)}
                                </pre>
                              </div>
                            )}

                            {/* Tool Call Output */}
                            {step.tool_output !== null && step.tool_output !== undefined && (
                              <div className="space-y-1">
                                <span className="text-[11px] text-neutral-400 font-medium block">
                                  Observation Output
                                </span>
                                <pre className="p-2 rounded bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 text-neutral-800 dark:text-neutral-200 text-[10px] font-mono overflow-x-auto max-h-36">
                                  {typeof step.tool_output === 'string'
                                    ? step.tool_output
                                    : JSON.stringify(step.tool_output, null, 2)}
                                </pre>
                              </div>
                            )}
                          </div>
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
};
