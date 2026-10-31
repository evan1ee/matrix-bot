import React, { useState } from 'react';
import { Calendar, Clock, Edit2, Loader2, Pause, Play, PlayCircle, Trash2 } from 'lucide-react';
import { useDeleteTask, useScheduledTasks, useSetTaskEnabled, useTriggerTask } from '../api';
import { ScheduledTask } from '../types';
import { TaskConfigModal } from './TaskConfigModal';
import { formatDateTimeInZone, useSettings } from '../../../core/settings';
import { showToast } from '../../../core/toast';

const BUTTON =
  'px-2.5 py-1.5 rounded-md border border-neutral-200 dark:border-neutral-700 hover:bg-neutral-50 dark:hover:bg-neutral-800 text-neutral-700 dark:text-neutral-300 text-[11px] font-medium inline-flex items-center gap-1.5 transition-colors duration-150 cursor-pointer focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-neutral-900 dark:focus-visible:outline-neutral-100 disabled:opacity-50';

export const ScheduledTasksTab: React.FC = () => {
  const { data: tasks = [], isLoading, isError } = useScheduledTasks();
  const setEnabled = useSetTaskEnabled();
  const trigger = useTriggerTask();
  const remove = useDeleteTask();
  const { timezone } = useSettings();

  const [configuringTask, setConfiguringTask] = useState<ScheduledTask | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState<string | null>(null);

  const reportError = (title: string, err: unknown) =>
    showToast({
      title,
      description: err instanceof Error ? err.message : 'Request failed',
      type: 'error',
    });

  const handleToggle = (task: ScheduledTask) =>
    setEnabled.mutate(
      { id: task.task_id, enabled: !task.enabled },
      { onError: (err) => reportError('Could not update task', err) }
    );

  const handleRunNow = (task: ScheduledTask) =>
    trigger.mutate(task.task_id, {
      onSuccess: () =>
        showToast({ title: 'Task started', description: task.task_id, type: 'success' }),
      onError: (err) => reportError('Could not start task', err),
    });

  const handleDelete = (task: ScheduledTask) =>
    remove.mutate(task.task_id, {
      onSuccess: () => setConfirmingDelete(null),
      onError: (err) => reportError('Could not delete task', err),
    });

  if (isLoading) {
    return (
      <div className="flex flex-col items-center justify-center py-20 text-neutral-400 gap-2">
        <Loader2 className="w-6 h-6 animate-spin" />
        <span>Loading scheduled tasks...</span>
      </div>
    );
  }

  if (isError) {
    return (
      <div className="p-4 rounded-lg bg-red-50 dark:bg-red-950/40 border border-red-200 dark:border-red-900 text-red-600 dark:text-red-400 text-xs">
        Failed to load scheduled agent tasks.
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100">
            Registered Autonomous Workflows
          </h3>
          <p className="text-xs text-neutral-500 dark:text-neutral-400">
            Recurring background tasks orchestrated by the PostgreSQL scheduler.
          </p>
        </div>
      </div>

      {tasks.length === 0 ? (
        <div className="py-12 text-center text-neutral-400 border border-dashed border-neutral-200 dark:border-neutral-800 rounded-xl text-xs">
          No scheduled tasks registered.
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          {tasks.map((task) => (
            <div
              key={task.id}
              className="p-4 rounded-xl border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900/60 shadow-xs flex flex-col justify-between gap-4 text-xs font-sans"
            >
              <div className="space-y-2">
                <div className="flex items-start justify-between gap-2">
                  <div>
                    <h4 className="font-semibold text-neutral-900 dark:text-neutral-100 font-mono text-xs">
                      {task.task_id}
                    </h4>
                    <span className="text-[11px] text-neutral-500 dark:text-neutral-400 block mt-0.5">
                      Autonomous Background Task
                    </span>
                  </div>

                  <span
                    className={`inline-flex items-center px-2 py-0.5 rounded-full text-[10px] font-medium ${
                      task.enabled
                        ? 'bg-emerald-100 dark:bg-emerald-950/60 text-emerald-800 dark:text-emerald-300'
                        : 'bg-neutral-100 dark:bg-neutral-800 text-neutral-500'
                    }`}
                  >
                    {task.enabled ? 'Active' : 'Paused'}
                  </span>
                </div>

                <div className="p-2.5 rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-100 dark:border-neutral-850 space-y-1.5 text-[11px]">
                  <div className="flex items-center justify-between">
                    <span className="text-neutral-400 flex items-center gap-1.5">
                      <Calendar size={12} />
                      Cron Expression:
                    </span>
                    <span className="font-mono font-medium text-neutral-800 dark:text-neutral-200">
                      {task.cron_expression}
                    </span>
                  </div>
                  <div className="flex items-center justify-between">
                    <span className="text-neutral-400 flex items-center gap-1.5">
                      <Clock size={12} />
                      Next Due:
                    </span>
                    <span className="font-mono text-neutral-700 dark:text-neutral-300">
                      {formatDateTimeInZone(task.next_run_at, timezone)}
                    </span>
                  </div>
                  <div className="flex items-center justify-between">
                    <span className="text-neutral-400 flex items-center gap-1.5">
                      <PlayCircle size={12} />
                      Last Run:
                    </span>
                    <span className="font-mono text-neutral-700 dark:text-neutral-300">
                      {task.last_run_at ? formatDateTimeInZone(task.last_run_at, timezone) : 'Never'}
                    </span>
                  </div>
                </div>
              </div>

              <div className="flex items-center justify-end gap-2 pt-2 border-t border-neutral-100 dark:border-neutral-800">
                {confirmingDelete === task.task_id ? (
                  <>
                    <span className="mr-auto text-[11px] text-neutral-500">Delete this task?</span>
                    <button type="button" onClick={() => setConfirmingDelete(null)} className={BUTTON}>
                      Cancel
                    </button>
                    <button
                      type="button"
                      onClick={() => handleDelete(task)}
                      disabled={remove.isPending}
                      className={`${BUTTON} text-red-600 dark:text-red-400`}
                    >
                      Confirm delete
                    </button>
                  </>
                ) : (
                  <>
                    <button
                      type="button"
                      onClick={() => handleToggle(task)}
                      disabled={setEnabled.isPending}
                      aria-label={`${task.enabled ? 'Pause' : 'Resume'} ${task.task_id}`}
                      className={BUTTON}
                    >
                      {task.enabled ? <Pause size={12} /> : <Play size={12} />}
                      {task.enabled ? 'Pause' : 'Resume'}
                    </button>
                    <button
                      type="button"
                      onClick={() => handleRunNow(task)}
                      disabled={trigger.isPending}
                      aria-label={`Run ${task.task_id} now`}
                      className={BUTTON}
                    >
                      {trigger.isPending && trigger.variables === task.task_id ? (
                        <Loader2 size={12} className="animate-spin" />
                      ) : (
                        <PlayCircle size={12} />
                      )}
                      Run now
                    </button>
                    <button type="button" onClick={() => setConfiguringTask(task)} className={BUTTON}>
                      <Edit2 size={12} />
                      Configure...
                    </button>
                    <button
                      type="button"
                      onClick={() => setConfirmingDelete(task.task_id)}
                      aria-label={`Delete ${task.task_id}`}
                      className={BUTTON}
                    >
                      <Trash2 size={12} />
                    </button>
                  </>
                )}
              </div>
            </div>
          ))}
        </div>
      )}

      {configuringTask && (
        <TaskConfigModal
          task={configuringTask}
          onClose={() => setConfiguringTask(null)}
        />
      )}
    </div>
  );
};
