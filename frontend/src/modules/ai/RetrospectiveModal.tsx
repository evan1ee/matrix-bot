import React, { useEffect, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import { WeeklyRetrospective } from './types';
import {
  Calendar, DollarSign, CheckCircle2, TrendingUp,
  Smile, Bell, Copy, Check, X, RefreshCw, Sparkles
} from 'lucide-react';

interface RetrospectiveModalProps {
  isOpen: boolean;
  onClose: () => void;
  onShareToChat?: (text: string) => void;
}

export const RetrospectiveModal: React.FC<RetrospectiveModalProps> = ({
  isOpen,
  onClose,
  onShareToChat,
}) => {
  const [copied, setCopied] = useState(false);

  const {
    data: retro,
    isLoading,
    isError,
    refetch,
    isRefetching,
  } = useQuery({
    queryKey: ['ai-retrospective'],
    queryFn: () => fetchApi<WeeklyRetrospective>('/ai/retrospective'),
    enabled: isOpen,
  });

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && isOpen) {
        onClose();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  const handleCopy = () => {
    if (!retro) return;
    navigator.clipboard.writeText(retro.formatted_markdown);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const spentDec = retro ? (retro.total_spent_minor / 100).toFixed(2) : '0.00';
  const incomeDec = retro ? (retro.total_income_minor / 100).toFixed(2) : '0.00';
  const netDec = retro ? (retro.net_cashflow_minor / 100).toFixed(2) : '0.00';
  const isNetPositive = (retro?.net_cashflow_minor ?? 0) >= 0;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/50 backdrop-blur-xs animate-in fade-in-0 duration-200"
      onClick={onClose}
    >
      <div
        className="relative w-full max-w-2xl max-h-[90vh] bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded-2xl shadow-xl flex flex-col overflow-hidden text-neutral-900 dark:text-neutral-100"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-900/50">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-md bg-neutral-100 dark:bg-neutral-800 text-neutral-700 dark:text-neutral-300 flex items-center justify-center shrink-0">
              <Calendar className="w-4 h-4" />
            </div>
            <div>
              <h2 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100 font-sans">Weekly Executive Retrospective</h2>
              <p className="text-xs text-neutral-500 dark:text-neutral-400 font-sans">
                {retro ? `${retro.period_start} to ${retro.period_end}` : '7-Day Review'}
              </p>
            </div>
          </div>
          <div className="flex items-center gap-1.5">
            <button
              type="button"
              onClick={() => refetch()}
              disabled={isRefetching || isLoading}
              title="Refresh Retrospective"
              className="p-1.5 text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-100 rounded hover:bg-neutral-100 dark:hover:bg-neutral-800 transition cursor-pointer"
            >
              <RefreshCw className={`w-4 h-4 ${isRefetching ? 'animate-spin' : ''}`} />
            </button>
            <button
              type="button"
              onClick={onClose}
              className="p-1.5 text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-100 rounded hover:bg-neutral-100 dark:hover:bg-neutral-800 transition cursor-pointer"
            >
              <X className="w-4 h-4" />
            </button>
          </div>
        </div>

        {/* Body Content */}
        <div className="flex-1 overflow-y-auto p-6 space-y-5">
          {isLoading ? (
            <div className="py-20 text-center text-xs font-mono text-neutral-500 dark:text-neutral-400 flex flex-col items-center gap-3">
              <RefreshCw className="w-5 h-5 animate-spin text-neutral-500" />
              <span>Analyzing your week across all modules...</span>
            </div>
          ) : isError || !retro ? (
            <div className="py-12 text-center text-xs font-sans text-red-500">
              Failed to load retrospective. Please try again.
            </div>
          ) : (
            <>
              {/* Stat Grid */}
              <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
                {/* Financial Velocity */}
                <div className="p-3.5 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-850/50 space-y-1">
                  <div className="flex items-center justify-between text-neutral-500 dark:text-neutral-400 text-[10px] font-mono uppercase tracking-wider">
                    <span>Net Flow</span>
                    <DollarSign className="w-3.5 h-3.5" />
                  </div>
                  <div className={`text-base font-semibold font-sans ${isNetPositive ? 'text-emerald-600 dark:text-emerald-400' : 'text-red-600 dark:text-red-400'}`}>
                    {retro.currency} {netDec}
                  </div>
                  <div className="text-[11px] font-mono text-neutral-500 truncate">
                    Spent {retro.currency} {spentDec}
                  </div>
                </div>

                {/* Tasks Completed */}
                <div className="p-3.5 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-850/50 space-y-1">
                  <div className="flex items-center justify-between text-neutral-500 dark:text-neutral-400 text-[10px] font-mono uppercase tracking-wider">
                    <span>Tasks</span>
                    <CheckCircle2 className="w-3.5 h-3.5" />
                  </div>
                  <div className="text-base font-semibold text-neutral-900 dark:text-neutral-100 font-sans">
                    {retro.completed_tasks_count} done
                  </div>
                  <div className="text-[11px] font-mono text-neutral-500 truncate">
                    {retro.active_tasks_count} remaining
                  </div>
                </div>

                {/* Habit Adherence */}
                <div className="p-3.5 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-850/50 space-y-1">
                  <div className="flex items-center justify-between text-neutral-500 dark:text-neutral-400 text-[10px] font-mono uppercase tracking-wider">
                    <span>Habits</span>
                    <TrendingUp className="w-3.5 h-3.5" />
                  </div>
                  <div className="text-base font-semibold text-neutral-900 dark:text-neutral-100 font-sans">
                    {retro.habits_adherence_percent}%
                  </div>
                  <div className="text-[11px] font-mono text-neutral-500 truncate">
                    {retro.active_habits_count} active
                  </div>
                </div>

                {/* Life & Wellness */}
                <div className="p-3.5 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-850/50 space-y-1">
                  <div className="flex items-center justify-between text-neutral-500 dark:text-neutral-400 text-[10px] font-mono uppercase tracking-wider">
                    <span>Mood</span>
                    <Smile className="w-3.5 h-3.5" />
                  </div>
                  <div className="text-base font-semibold capitalize text-neutral-900 dark:text-neutral-100 font-sans truncate">
                    {retro.dominant_mood || 'None'}
                  </div>
                  <div className="text-[11px] font-mono text-neutral-500 truncate">
                    {retro.days_logged_count}/7 logged
                  </div>
                </div>
              </div>

              {/* Upcoming Highlights */}
              <div className="p-3 rounded-lg bg-neutral-50 dark:bg-neutral-900/60 border border-neutral-200 dark:border-neutral-800 text-xs text-neutral-800 dark:text-neutral-200 flex items-center justify-between font-sans">
                <div className="flex items-center gap-2">
                  <Bell className="w-4 h-4 text-neutral-500 shrink-0" />
                  <span>
                    Upcoming (Next 7 Days): <strong className="font-semibold">{retro.upcoming_bills_count} bills</strong> ({retro.currency} {(retro.upcoming_bills_total_minor / 100).toFixed(2)}) &amp; <strong className="font-semibold">{retro.upcoming_reminders_count} reminders</strong>
                  </span>
                </div>
              </div>

              {/* Full Formatted Summary */}
              <div className="rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-900/30 p-4 space-y-2">
                <h4 className="text-[10px] font-mono uppercase tracking-wider text-neutral-500">
                  Consolidated Brief
                </h4>
                <div className="text-xs font-mono whitespace-pre-wrap leading-relaxed text-neutral-800 dark:text-neutral-200 bg-white dark:bg-neutral-900 p-3.5 rounded border border-neutral-200 dark:border-neutral-800">
                  {retro.formatted_markdown}
                </div>
              </div>
            </>
          )}
        </div>

        {/* Footer Actions */}
        <div className="flex items-center justify-between px-6 py-3.5 border-t border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-900/50">
          <button
            type="button"
            onClick={handleCopy}
            disabled={!retro}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded border border-neutral-200 dark:border-neutral-800 hover:bg-neutral-100 dark:hover:bg-neutral-800 text-neutral-700 dark:text-neutral-200 transition cursor-pointer"
          >
            {copied ? <Check className="w-3.5 h-3.5 text-emerald-600" /> : <Copy className="w-3.5 h-3.5" />}
            <span>{copied ? 'Copied!' : 'Copy Summary'}</span>
          </button>

          <div className="flex items-center gap-2">
            {onShareToChat && (
              <button
                type="button"
                onClick={() => {
                  if (retro) {
                    onShareToChat('Please analyze my weekly retrospective and suggest 3 high-impact focus areas for next week.');
                    onClose();
                  }
                }}
                disabled={!retro}
                className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium bg-neutral-900 hover:bg-neutral-800 text-white dark:bg-neutral-100 dark:hover:bg-neutral-200 dark:text-neutral-900 rounded transition cursor-pointer"
              >
                <Sparkles className="w-3.5 h-3.5" />
                <span>Ask AI to Plan Next Week</span>
              </button>
            )}
            <button
              type="button"
              onClick={onClose}
              className="px-3 py-1.5 text-xs font-medium text-neutral-600 hover:text-neutral-900 dark:text-neutral-400 dark:hover:text-neutral-100 rounded border border-neutral-200 dark:border-neutral-800 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition cursor-pointer"
            >
              Close
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};
