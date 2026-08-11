import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from './apiClient';
import { signInWithPasskey } from './passkey';

export interface Passkey {
  id: string;
  label: string;
  created_at: string | null;
  last_used_at: string | null;
}

export interface DeviceSession {
  id: string;
  created_at: string;
  last_seen_at: string;
  user_agent: string | null;
  current: boolean;
}

export interface MergePreview {
  todos: number;
  reminders: number;
  transactions: number;
  recurring_bills: number;
  portfolio_holdings: number;
  portfolio_cash_balances: number;
  finance_categories: number;
  user_settings: number;
}

export interface PendingMerge {
  from_account: string;
  preview: MergePreview;
}

export interface SecurityEvent {
  id: number;
  kind: string;
  detail: string | null;
  created_at: string | null;
}

export interface RecoveryStatus {
  codes_remaining: number;
  recovery_valid_after: string | null;
}

export const SECURITY_KEYS = {
  passkeys: ['account', 'passkeys'],
  links: ['account', 'matrix'],
  sessions: ['account', 'sessions'],
  events: ['account', 'security-events'],
  recovery: ['account', 'recovery-status'],
  pendingMerge: ['account', 'pending-merge'],
};

export function usePasskeys() {
  return useQuery<Passkey[]>({
    queryKey: SECURITY_KEYS.passkeys,
    queryFn: () => fetchApi<Passkey[]>('/account/passkeys'),
  });
}

export function useLinkedAccounts() {
  return useQuery<string[]>({
    queryKey: SECURITY_KEYS.links,
    queryFn: () => fetchApi<string[]>('/account/matrix'),
  });
}

export function useDeviceSessions() {
  return useQuery<DeviceSession[]>({
    queryKey: SECURITY_KEYS.sessions,
    queryFn: () => fetchApi<DeviceSession[]>('/account/sessions'),
  });
}

export function useSecurityEvents() {
  return useQuery<SecurityEvent[]>({
    queryKey: SECURITY_KEYS.events,
    queryFn: () => fetchApi<SecurityEvent[]>('/account/security-events'),
  });
}

export function useRecoveryStatus() {
  return useQuery<RecoveryStatus>({
    queryKey: SECURITY_KEYS.recovery,
    queryFn: () => fetchApi<RecoveryStatus>('/account/recovery-status'),
  });
}

export function usePendingMerge() {
  return useQuery<PendingMerge | null>({
    queryKey: SECURITY_KEYS.pendingMerge,
    queryFn: () => fetchApi<PendingMerge | null>('/account/matrix/pending-merge'),
    // A merge is announced by a Matrix DM, so the tab may have been open the
    // whole time it happened. Poll while this screen is showing.
    refetchInterval: 10_000,
  });
}

/**
 * Runs a sensitive action, re-proving possession of a passkey first if the
 * server says the step-up window has lapsed.
 *
 * The 403 is not an error to show the user — it is the server asking for a
 * passkey, so the prompt appears and the action is retried. Only a second
 * failure is worth reporting.
 */
export function useSensitiveAction<TArgs, TResult>(
  action: (args: TArgs) => Promise<TResult>,
  invalidate: (string[] | readonly string[])[] = []
) {
  const queryClient = useQueryClient();

  return useMutation<TResult, Error, TArgs>({
    mutationFn: async (args: TArgs) => {
      try {
        return await action(args);
      } catch (err) {
        if (!(err instanceof Error) || !isStepUpRequired(err)) throw err;
        await signInWithPasskey();
        return action(args);
      }
    },
    onSuccess: () => {
      for (const key of invalidate) {
        queryClient.invalidateQueries({ queryKey: key as string[] });
      }
    },
  });
}

/** Whether a failure was the server asking for a fresh passkey. */
function isStepUpRequired(error: Error): boolean {
  return /passkey to continue/i.test(error.message);
}

export const securityApi = {
  startLink: () =>
    fetchApi<{ code: string; expires_in_seconds: number }>('/account/matrix/link', {
      method: 'POST',
    }),
  unlink: (mxid: string) =>
    fetchApi<void>('/account/matrix/unlink', {
      method: 'POST',
      body: JSON.stringify({ mxid }),
    }),
  confirmMerge: (fromAccount: string) =>
    fetchApi<MergePreview>('/account/matrix/confirm-merge', {
      method: 'POST',
      body: JSON.stringify({ from_account: fromAccount }),
    }),
  deletePasskey: (id: string) =>
    fetchApi<void>(`/account/passkeys/${encodeURIComponent(id)}`, { method: 'DELETE' }),
  renamePasskey: ({ id, label }: { id: string; label: string }) =>
    fetchApi<void>(`/account/passkeys/${encodeURIComponent(id)}`, {
      method: 'PUT',
      body: JSON.stringify({ label }),
    }),
  revokeOtherSessions: () =>
    fetchApi<number>('/account/sessions/revoke-others', { method: 'POST' }),
  regenerateRecoveryCodes: () =>
    fetchApi<{ codes: string[] }>('/account/recovery-codes', { method: 'POST' }),
  cancelRecovery: () => fetchApi<number>('/account/recovery/cancel', { method: 'POST' }),
};

/** Turns a `security_events.kind` into something a person reads. */
export function describeEvent(kind: string): string {
  const labels: Record<string, string> = {
    signed_in: 'Signed in',
    passkey_added: 'Passkey added',
    passkey_removed: 'Passkey removed',
    passkey_renamed: 'Passkey renamed',
    matrix_linked: 'Matrix account linked',
    matrix_unlinked: 'Matrix account unlinked',
    accounts_merged: 'Data merged in',
    recovery_codes_issued: 'Recovery codes generated',
    recovery_code_used: 'Recovery code used',
    recovery_requested: 'Account recovery requested',
    recovery_cancelled: 'Account recovery cancelled',
    recovery_completed: 'Account recovery completed',
    sessions_revoked: 'Other devices signed out',
    financial_data_exported: 'Financial data exported',
  };
  return labels[kind] ?? kind.replace(/_/g, ' ');
}

/** Whether an event should be shown as a warning rather than as history. */
export function isAlarming(kind: string): boolean {
  return kind.startsWith('recovery_') && kind !== 'recovery_cancelled';
}
