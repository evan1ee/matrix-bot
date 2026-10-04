import React, { useState } from 'react';
import { Calendar, Check, Loader2, X } from 'lucide-react';
import { ScheduledTask } from '../types';
import { useUpdateScheduledTask } from '../api';

interface TaskConfigModalProps {
  task: ScheduledTask | null;
  onClose: () => void;
}

export const TaskConfigModal: React.FC<TaskConfigModalProps> = ({ task, onClose }) => {
  const updateMutation = useUpdateScheduledTask();

  const [cronExpression, setCronExpression] = useState(task?.cron_expression ?? '');
  const [configJson, setConfigJson] = useState(() =>
    task?.config ? JSON.stringify(task.config, null, 2) : '{}'
  );
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  if (!task) return null;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const trimmedCron = cronExpression.trim();
    if (!trimmedCron) {
      setErrorMsg('Cron expression cannot be empty.');
      return;
    }

    let parsedConfig: unknown = {};
    if (configJson.trim()) {
      try {
        parsedConfig = JSON.parse(configJson);
      } catch (err) {
        setErrorMsg('Invalid JSON configuration syntax.');
        return;
      }
    }

    setErrorMsg(null);
    try {
      await updateMutation.mutateAsync({
        id: task.task_id,
        data: {
          cron_expression: trimmedCron,
          config: parsedConfig,
        },
      });
      onClose();
    } catch (err) {
      setErrorMsg(err instanceof Error ? err.message : 'Failed to update scheduled task');
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
            <Calendar className="w-4 h-4 text-neutral-700 dark:text-neutral-300" />
            <h3 className="text-sm font-semibold">Configure Scheduled Task</h3>
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
            <label className="block text-xs font-medium text-neutral-500 mb-1">
              Task Identifier
            </label>
            <div className="px-3 py-2 rounded-md bg-neutral-100 dark:bg-neutral-800 font-mono text-neutral-900 dark:text-neutral-100 text-xs">
              {task.task_id}
            </div>
          </div>

          <div>
            <label htmlFor="task-cron" className="block text-xs font-medium text-neutral-700 dark:text-neutral-300 mb-1">
              Cron Expression
            </label>
            <input
              id="task-cron"
              type="text"
              value={cronExpression}
              onChange={(e) => setCronExpression(e.target.value)}
              placeholder="e.g. 0 9 * * 1 (Every Monday at 9:00 AM)"
              className="w-full px-3 py-2 rounded-md bg-white dark:bg-neutral-950 border border-neutral-300 dark:border-neutral-700 font-mono text-xs focus:outline-none focus:ring-1 focus:ring-neutral-400"
            />
            <span className="text-[10px] text-neutral-500 mt-1 block">
              Standard 5-part expression: min hour day-of-month month day-of-week
            </span>
          </div>

          <div>
            <label htmlFor="task-config" className="block text-xs font-medium text-neutral-700 dark:text-neutral-300 mb-1">
              Custom Configuration JSON
            </label>
            <textarea
              id="task-config"
              value={configJson}
              onChange={(e) => setConfigJson(e.target.value)}
              rows={5}
              className="w-full px-3 py-2 rounded-md bg-white dark:bg-neutral-950 border border-neutral-300 dark:border-neutral-700 font-mono text-xs focus:outline-none focus:ring-1 focus:ring-neutral-400 resize-none"
            />
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
              disabled={updateMutation.isPending || !cronExpression.trim()}
              className="px-4 py-2 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 hover:bg-neutral-800 dark:hover:bg-neutral-200 font-medium inline-flex items-center gap-1.5 transition-colors cursor-pointer"
            >
              {updateMutation.isPending ? (
                <>
                  <Loader2 size={14} className="animate-spin" />
                  Saving...
                </>
              ) : (
                <>
                  <Check size={14} />
                  Save Changes
                </>
              )}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
