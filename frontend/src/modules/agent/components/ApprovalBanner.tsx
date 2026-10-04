import React, { useState } from 'react';
import { AlertTriangle, Check, Loader2, ShieldAlert, X } from 'lucide-react';
import { useAgentApprovals, useApproveAction, useRejectAction } from '../api';
import { ApprovalRequest } from '../types';
import { formatDateTimeInZone, useSettings } from '../../../core/settings';

export const ApprovalBanner: React.FC = () => {
  const { data: approvals = [] } = useAgentApprovals();
  const pendingApprovals = approvals.filter((a) => a.status === 'pending');
  const [selectedApproval, setSelectedApproval] = useState<ApprovalRequest | null>(null);
  const [rejectReason, setRejectReason] = useState('');
  const [isRejecting, setIsRejecting] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const { timezone } = useSettings();
  const approveMutation = useApproveAction();
  const rejectMutation = useRejectAction();

  if (pendingApprovals.length === 0) return null;

  const handleApprove = async (approval: ApprovalRequest) => {
    setErrorMsg(null);
    try {
      await approveMutation.mutateAsync({ id: approval.id });
      setSelectedApproval(null);
    } catch (err) {
      setErrorMsg(err instanceof Error ? err.message : 'Failed to approve action');
    }
  };

  const handleReject = async (approval: ApprovalRequest) => {
    setErrorMsg(null);
    try {
      await rejectMutation.mutateAsync({
        id: approval.id,
        reason: rejectReason.trim() || undefined,
      });
      setSelectedApproval(null);
      setRejectReason('');
      setIsRejecting(false);
    } catch (err) {
      setErrorMsg(err instanceof Error ? err.message : 'Failed to reject action');
    }
  };

  return (
    <>
      {/* Banner on top of Agent Module */}
      <div className="flex items-center justify-between gap-4 p-4 rounded-lg bg-amber-50 dark:bg-amber-950/40 border border-amber-200 dark:border-amber-800 text-amber-900 dark:text-amber-100 animate-in fade-in duration-150">
        <div className="flex items-center gap-3 min-w-0">
          <ShieldAlert className="w-5 h-5 text-amber-600 dark:text-amber-400 shrink-0" />
          <div className="text-xs">
            <span className="font-semibold">
              {pendingApprovals.length === 1
                ? '1 action requires approval'
                : `${pendingApprovals.length} actions require approval`}
            </span>
            <span className="hidden sm:inline text-amber-700 dark:text-amber-300 ml-2">
              The agent is paused waiting for your authorization to execute a sensitive tool.
            </span>
          </div>
        </div>

        <button
          type="button"
          onClick={() => {
            setSelectedApproval(pendingApprovals[0]);
            setErrorMsg(null);
          }}
          className="shrink-0 px-3 py-1.5 rounded-md bg-amber-600 hover:bg-amber-700 text-white text-xs font-medium transition-colors cursor-pointer"
        >
          Review {pendingApprovals.length > 1 ? `(${pendingApprovals.length})` : 'Action'}
        </button>
      </div>

      {/* Review Modal */}
      {selectedApproval && (
        <div
          role="dialog"
          aria-modal="true"
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/50 backdrop-blur-xs animate-in fade-in duration-150"
          onKeyDown={(e) => {
            if (e.key === 'Escape') setSelectedApproval(null);
          }}
        >
          <div className="relative w-full max-w-lg rounded-xl bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 shadow-2xl p-6 flex flex-col gap-4 text-xs font-sans text-neutral-900 dark:text-neutral-100">
            {/* Modal Header */}
            <div className="flex items-center justify-between border-b border-neutral-100 dark:border-neutral-800 pb-3">
              <div className="flex items-center gap-2">
                <AlertTriangle className="w-4 h-4 text-amber-500" />
                <h3 className="text-sm font-semibold">Authorize Sensitive Agent Action</h3>
              </div>
              <button
                type="button"
                onClick={() => setSelectedApproval(null)}
                className="text-neutral-400 hover:text-neutral-600 dark:hover:text-neutral-200 p-1 rounded"
              >
                <X size={16} />
              </button>
            </div>

            {/* Context & Description */}
            <div className="space-y-3">
              <p className="text-neutral-600 dark:text-neutral-400 leading-relaxed">
                The agent has proposed the following sensitive action. Approval requires biometric / passkey verification.
              </p>

              <div className="p-3 rounded-lg bg-neutral-50 dark:bg-neutral-950 border border-neutral-200 dark:border-neutral-800 space-y-2">
                <div className="flex items-center justify-between">
                  <span className="text-neutral-500 font-medium">Tool Name:</span>
                  <span className="font-mono font-semibold px-2 py-0.5 rounded bg-neutral-200 dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100">
                    {selectedApproval.tool_name}
                  </span>
                </div>
                <div className="flex items-center justify-between">
                  <span className="text-neutral-500 font-medium">Expires At:</span>
                  <span className="font-mono text-neutral-700 dark:text-neutral-300">
                    {formatDateTimeInZone(selectedApproval.expires_at, timezone)}
                  </span>
                </div>
                <div>
                  <span className="text-neutral-500 font-medium block mb-1">Parameters:</span>
                  <pre className="p-2.5 rounded bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-[11px] font-mono overflow-x-auto text-neutral-800 dark:text-neutral-200 max-h-48">
                    {JSON.stringify(selectedApproval.tool_input, null, 2)}
                  </pre>
                </div>
              </div>

              {errorMsg && (
                <div className="p-2.5 rounded bg-red-50 dark:bg-red-950/40 border border-red-200 dark:border-red-900 text-red-600 dark:text-red-400">
                  {errorMsg}
                </div>
              )}

              {isRejecting ? (
                <div className="space-y-2 pt-2 border-t border-neutral-100 dark:border-neutral-800">
                  <label htmlFor="reject-reason" className="block text-xs font-medium text-neutral-700 dark:text-neutral-300">
                    Rejection Reason (Optional):
                  </label>
                  <input
                    id="reject-reason"
                    type="text"
                    value={rejectReason}
                    onChange={(e) => setRejectReason(e.target.value)}
                    placeholder="e.g. Too expensive or incorrect parameters"
                    className="w-full px-3 py-2 rounded-md bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-700 text-xs focus:outline-none focus:ring-1 focus:ring-neutral-400"
                    autoFocus
                  />
                  <div className="flex items-center justify-end gap-2 pt-1">
                    <button
                      type="button"
                      onClick={() => setIsRejecting(false)}
                      className="px-3 py-1.5 rounded text-neutral-600 dark:text-neutral-400 hover:bg-neutral-100 dark:hover:bg-neutral-800"
                    >
                      Back
                    </button>
                    <button
                      type="button"
                      disabled={rejectMutation.isPending}
                      onClick={() => handleReject(selectedApproval)}
                      className="px-3 py-1.5 rounded bg-red-600 hover:bg-red-700 text-white font-medium inline-flex items-center gap-1.5"
                    >
                      {rejectMutation.isPending && <Loader2 size={13} className="animate-spin" />}
                      Confirm Rejection
                    </button>
                  </div>
                </div>
              ) : null}
            </div>

            {/* Modal Actions */}
            {!isRejecting && (
              <div className="flex items-center justify-end gap-2 border-t border-neutral-100 dark:border-neutral-800 pt-3">
                <button
                  type="button"
                  onClick={() => setIsRejecting(true)}
                  className="px-3 py-2 rounded-md border border-neutral-300 dark:border-neutral-700 text-neutral-700 dark:text-neutral-300 hover:bg-neutral-50 dark:hover:bg-neutral-800 font-medium transition-colors"
                >
                  Reject...
                </button>
                <button
                  type="button"
                  disabled={approveMutation.isPending}
                  onClick={() => handleApprove(selectedApproval)}
                  className="px-4 py-2 rounded-md bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 hover:bg-neutral-800 dark:hover:bg-neutral-200 font-medium inline-flex items-center gap-1.5 transition-colors cursor-pointer"
                >
                  {approveMutation.isPending ? (
                    <>
                      <Loader2 size={14} className="animate-spin" />
                      Verifying...
                    </>
                  ) : (
                    <>
                      <Check size={14} />
                      Approve Action
                    </>
                  )}
                </button>
              </div>
            )}
          </div>
        </div>
      )}
    </>
  );
};
