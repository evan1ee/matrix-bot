import React from 'react';
import { AlertCircle, AlertTriangle, CheckCircle, Info, X } from 'lucide-react';
import { dismissToast, Toast, useToasts } from '../core/toast';

export const ToastContainer: React.FC = () => {
  const toasts = useToasts();

  if (toasts.length === 0) return null;

  return (
    <div
      aria-live="polite"
      className="fixed bottom-5 right-5 z-50 flex flex-col gap-2 max-w-sm w-full pointer-events-none"
    >
      {toasts.map((toast) => {
        const Icon =
          toast.type === 'error'
            ? AlertCircle
            : toast.type === 'warning'
            ? AlertTriangle
            : toast.type === 'success'
            ? CheckCircle
            : Info;

        const iconColor =
          toast.type === 'error'
            ? 'text-red-500'
            : toast.type === 'warning'
            ? 'text-amber-500'
            : toast.type === 'success'
            ? 'text-emerald-500'
            : 'text-neutral-400';

        return (
          <div
            key={toast.id}
            role="status"
            className="pointer-events-auto flex items-start gap-3 p-3.5 bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded-lg shadow-lg text-xs transition-all animate-in fade-in slide-in-from-bottom-2 duration-150"
          >
            <Icon size={16} className={`shrink-0 mt-0.5 ${iconColor}`} />
            <div className="flex-1 min-w-0 flex flex-col gap-1">
              <div className="font-semibold text-neutral-900 dark:text-neutral-100 leading-snug">
                {toast.title}
              </div>
              {toast.description && (
                <div className="text-neutral-600 dark:text-neutral-400 leading-normal break-words">
                  {toast.description}
                </div>
              )}
              {toast.action && (
                <div className="mt-1.5">
                  <button
                    type="button"
                    onClick={() => {
                      toast.action?.onClick();
                      dismissToast(toast.id);
                    }}
                    className="inline-flex items-center px-2 py-1 rounded bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 font-medium hover:bg-neutral-800 dark:hover:bg-neutral-200 transition-colors"
                  >
                    {toast.action.label}
                  </button>
                </div>
              )}
            </div>
            <button
              type="button"
              onClick={() => dismissToast(toast.id)}
              className="text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-200 p-0.5 rounded transition-colors"
              aria-label="Dismiss notification"
            >
              <X size={14} />
            </button>
          </div>
        );
      })}
    </div>
  );
};
