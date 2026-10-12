import React, { useEffect } from 'react';
import { History, RefreshCw, X, FileText, Clock, AlertTriangle, CheckCircle2, AlertCircle } from 'lucide-react';
import { formatDateTimeInZone, useSettings } from '../../core/settings';
import { WebhookItem, WebhookLogsResponse } from './types';

interface WebhookLogsModalProps {
  webhook: WebhookItem;
  roomName: string;
  logsData?: WebhookLogsResponse;
  isLoading: boolean;
  onRefresh: () => void;
  onClose: () => void;
}

export const WebhookLogsModal: React.FC<WebhookLogsModalProps> = ({
  webhook,
  roomName,
  logsData,
  isLoading,
  onRefresh,
  onClose,
}) => {
  const { timezone } = useSettings();
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        onClose();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [onClose]);

  const getSeverityBadge = (severity: string) => {
    switch (severity.toLowerCase()) {
      case 'critical':
        return (
          <span className="inline-flex items-center gap-1 px-1.5 py-0.5 text-[10px] font-mono font-medium rounded bg-red-500/15 text-red-600 dark:text-red-400 border border-red-500/20">
            Critical
          </span>
        );
      case 'warning':
        return (
          <span className="inline-flex items-center gap-1 px-1.5 py-0.5 text-[10px] font-mono font-medium rounded bg-amber-500/15 text-amber-600 dark:text-amber-400 border border-amber-500/20">
            Warning
          </span>
        );
      case 'success':
        return (
          <span className="inline-flex items-center gap-1 px-1.5 py-0.5 text-[10px] font-mono font-medium rounded bg-emerald-500/15 text-emerald-600 dark:text-emerald-400 border border-emerald-500/20">
            Success
          </span>
        );
      default:
        return (
          <span className="inline-flex items-center gap-1 px-1.5 py-0.5 text-[10px] font-mono font-medium rounded bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-300 border border-neutral-200 dark:border-neutral-700">
            Info
          </span>
        );
    }
  };

  const getStatusBadge = (status: string) => {
    switch (status.toLowerCase()) {
      case 'delivered':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 text-[11px] font-medium rounded bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border border-emerald-500/20">
            <CheckCircle2 className="w-3 h-3" /> Delivered
          </span>
        );
      case 'failed':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 text-[11px] font-medium rounded bg-destructive/10 text-destructive border border-destructive/20">
            <AlertCircle className="w-3 h-3" /> Failed
          </span>
        );
      default:
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 text-[11px] font-medium rounded bg-amber-500/10 text-amber-600 dark:text-amber-400 border border-amber-500/20">
            <Clock className="w-3 h-3" /> Queued
          </span>
        );
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 bg-black/60 dark:bg-black/80 flex items-center justify-center p-3 sm:p-4"
      role="dialog"
      aria-modal="true"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="bg-white dark:bg-neutral-950 text-neutral-900 dark:text-neutral-100 w-full max-w-3xl rounded-xl border border-neutral-200 dark:border-neutral-800 shadow-2xl overflow-hidden animate-in fade-in-0 zoom-in-95 max-h-[88vh] flex flex-col">
        <div className="p-4 sm:p-5 border-b border-neutral-200 dark:border-neutral-800 flex items-center justify-between gap-3 bg-neutral-50 dark:bg-neutral-900/50">
          <div className="flex items-center gap-2.5 min-w-0">
            <History className="w-5 h-5 text-neutral-500 shrink-0" />
            <div className="min-w-0">
              <h2 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100 truncate font-sans">
                Delivery logs: {webhook.name}
              </h2>
              <p className="text-xs text-neutral-500 dark:text-neutral-400 truncate">
                Target Room: {roomName} &bull; {logsData?.total ?? 0} total events
              </p>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <button
              onClick={onRefresh}
              title="Refresh logs"
              className="p-1.5 text-neutral-500 hover:text-neutral-900 dark:text-neutral-400 dark:hover:text-neutral-100 bg-neutral-100 hover:bg-neutral-200 dark:bg-neutral-800 dark:hover:bg-neutral-700 rounded-lg transition-colors cursor-pointer"
            >
              <RefreshCw className={`w-4 h-4 ${isLoading ? 'animate-spin' : ''}`} />
            </button>
            <button
              onClick={onClose}
              title="Close logs"
              className="p-1.5 text-neutral-500 hover:text-neutral-900 dark:text-neutral-400 dark:hover:text-neutral-100 bg-neutral-100 hover:bg-neutral-200 dark:bg-neutral-800 dark:hover:bg-neutral-700 rounded-lg transition-colors cursor-pointer"
            >
              <X className="w-4 h-4" />
            </button>
          </div>
        </div>

        <div className="p-4 sm:p-6 overflow-y-auto space-y-3.5 flex-1">
          {isLoading ? (
            <div className="py-12 text-center text-sm text-neutral-500 dark:text-neutral-400">
              Loading message history...
            </div>
          ) : !logsData || logsData.items.length === 0 ? (
            <div className="py-12 text-center text-neutral-500 dark:text-neutral-400 space-y-2">
              <FileText className="w-8 h-8 mx-auto text-neutral-400 dark:text-neutral-600" />
              <p className="text-sm font-medium text-neutral-900 dark:text-neutral-100">No messages received yet</p>
              <p className="text-xs max-w-sm mx-auto">
                Messages and alerts sent to this webhook will be saved and displayed here.
              </p>
            </div>
          ) : (
            logsData.items.map((item) => (
              <div
                key={item.id}
                className="p-4 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/80 dark:bg-neutral-900/80 space-y-2.5 shadow-2xs"
              >
                <div className="flex items-center justify-between gap-2 flex-wrap">
                  <div className="flex items-center gap-2 flex-wrap">
                    {getStatusBadge(item.status)}
                    {getSeverityBadge(item.severity)}
                    {item.title && (
                      <span className="font-semibold text-xs text-neutral-900 dark:text-neutral-100">{item.title}</span>
                    )}
                  </div>
                  <div className="text-[11px] text-neutral-500 dark:text-neutral-400 flex items-center gap-1">
                    <Clock className="w-3 h-3" />
                    {formatDateTimeInZone(item.created_at, timezone)}
                  </div>
                </div>

                <div className="p-2.5 bg-white dark:bg-neutral-950 rounded-md text-xs font-mono text-neutral-900 dark:text-neutral-100 whitespace-pre-wrap break-words border border-neutral-200 dark:border-neutral-800 max-h-48 overflow-y-auto">
                  {item.plain_body}
                </div>

                {item.last_error && (
                  <div className="p-2.5 rounded-md bg-red-50 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 text-red-800 dark:text-red-300 text-xs flex items-start gap-2">
                    <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-0.5 text-red-500" />
                    <div>
                      <strong>Error (Attempt {item.attempts}/{item.max_attempts}):</strong> {item.last_error}
                    </div>
                  </div>
                )}
              </div>
            ))
          )}
        </div>

        <div className="p-3.5 border-t border-neutral-200 dark:border-neutral-800 bg-neutral-50 dark:bg-neutral-900/50 flex justify-end">
          <button
            onClick={onClose}
            className="px-4 py-1.5 text-xs font-medium bg-neutral-200 hover:bg-neutral-300 dark:bg-neutral-800 dark:hover:bg-neutral-700 text-neutral-800 dark:text-neutral-200 rounded-lg transition-colors cursor-pointer"
          >
            Close
          </button>
        </div>
      </div>
    </div>
  );
};
