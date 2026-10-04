import React, { useState } from 'react';
import { Loader2, Play, Sparkles, X } from 'lucide-react';
import { useStartRun } from '../api';

interface StartRunModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSuccess?: (runId: string) => void;
}

export const StartRunModal: React.FC<StartRunModalProps> = ({ isOpen, onClose, onSuccess }) => {
  const [goal, setGoal] = useState('');
  const [maxSteps, setMaxSteps] = useState(10);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const startRunMutation = useStartRun();

  if (!isOpen) return null;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const trimmedGoal = goal.trim();
    if (!trimmedGoal) {
      setErrorMsg('Please formulate an objective for the agent.');
      return;
    }

    setErrorMsg(null);
    try {
      const created = await startRunMutation.mutateAsync({
        goal: trimmedGoal,
        max_steps: maxSteps,
        origin_conversation: 'web',
      });
      setGoal('');
      setMaxSteps(10);
      onClose();
      onSuccess?.(created.id);
    } catch (err) {
      setErrorMsg(err instanceof Error ? err.message : 'Failed to launch agent run');
    }
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/50 backdrop-blur-xs animate-in fade-in duration-150"
      onKeyDown={(e) => {
        if (e.key === 'Escape') onClose();
      }}
    >
      <div className="relative w-full max-w-lg rounded-xl bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 shadow-2xl p-6 flex flex-col gap-4 text-xs font-sans text-neutral-900 dark:text-neutral-100">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-neutral-100 dark:border-neutral-800 pb-3">
          <div className="flex items-center gap-2">
            <Sparkles className="w-4 h-4 text-neutral-700 dark:text-neutral-300" />
            <h3 className="text-sm font-semibold">Start Autonomous Agent Goal</h3>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-200 p-1 rounded"
          >
            <X size={16} />
          </button>
        </div>

        <form onSubmit={handleSubmit} className="space-y-4">
          <div>
            <label htmlFor="agent-goal" className="block text-xs font-medium text-neutral-700 dark:text-neutral-300 mb-1">
              Goal & Instructions
            </label>
            <textarea
              id="agent-goal"
              value={goal}
              onChange={(e) => setGoal(e.target.value)}
              placeholder="e.g. Inspect my uncompleted high-priority tasks, summarize overdue reminders, and draft a plan for today."
              rows={4}
              maxLength={1000}
              className="w-full px-3 py-2 rounded-md bg-white dark:bg-neutral-950 border border-neutral-300 dark:border-neutral-700 text-xs focus:outline-none focus:ring-1 focus:ring-neutral-400 resize-none leading-relaxed"
              autoFocus
            />
            <div className="text-right text-[10px] text-neutral-400 mt-0.5">
              {goal.length} / 1000
            </div>
          </div>

          <div className="grid grid-cols-2 gap-4">
            <div>
              <label htmlFor="agent-max-steps" className="block text-xs font-medium text-neutral-700 dark:text-neutral-300 mb-1">
                Max Steps Budget
              </label>
              <input
                id="agent-max-steps"
                type="number"
                min={1}
                max={15}
                value={maxSteps}
                onChange={(e) => setMaxSteps(Math.max(1, Math.min(15, parseInt(e.target.value, 10) || 10)))}
                className="w-full px-3 py-2 rounded-md bg-white dark:bg-neutral-950 border border-neutral-300 dark:border-neutral-700 text-xs focus:outline-none focus:ring-1 focus:ring-neutral-400"
              />
              <span className="text-[10px] text-neutral-500 mt-1 block">
                Safety limit (1 to 15 steps)
              </span>
            </div>

            <div>
              <label className="block text-xs font-medium text-neutral-700 dark:text-neutral-300 mb-1">
                Execution Mode
              </label>
              <div className="px-3 py-2 rounded-md bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400 text-xs">
                Asynchronous Job
              </div>
              <span className="text-[10px] text-neutral-500 mt-1 block">
                Runs via background worker
              </span>
            </div>
          </div>

          {errorMsg && (
            <div className="p-2.5 rounded bg-red-50 dark:bg-red-950/40 border border-red-200 dark:border-red-900 text-red-600 dark:text-red-400">
              {errorMsg}
            </div>
          )}

          <div className="flex items-center justify-end gap-2 border-t border-neutral-100 dark:border-neutral-800 pt-3">
            <button
              type="button"
              onClick={onClose}
              className="px-3 py-2 rounded-md border border-neutral-300 dark:border-neutral-700 text-neutral-700 dark:text-neutral-300 hover:bg-neutral-50 dark:hover:bg-neutral-800 font-medium transition-colors"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={startRunMutation.isPending || !goal.trim()}
              className="px-4 py-2 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 hover:bg-neutral-800 dark:hover:bg-neutral-200 font-medium inline-flex items-center gap-1.5 transition-colors disabled:opacity-50 cursor-pointer"
            >
              {startRunMutation.isPending ? (
                <>
                  <Loader2 size={14} className="animate-spin" />
                  Dispatching...
                </>
              ) : (
                <>
                  <Play size={14} />
                  Start Goal
                </>
              )}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
