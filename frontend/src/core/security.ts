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

export interface LinkedIdentity {
  channel: string;
  user_id: string;
  display_name: string | null;
  linked_at: string;
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
  identities: ['account', 'identities'],
  sessions: ['account', 'sessions'],
  events: ['account', 'security-events'],
  recovery: ['account', 'recovery-status'],
};

export function usePasskeys() {
  return useQuery<Passkey[]>({
    queryKey: SECURITY_KEYS.passkeys,
    queryFn: () => fetchApi<Passkey[]>('/account/passkeys'),
  });
}

/** The Matrix user ids linked to this account, oldest first. */
export function useMatrixIdentities() {
  return useQuery<LinkedIdentity[]>({
    queryKey: SECURITY_KEYS.identities,
    queryFn: async () =>
      (await fetchApi<{ identities: LinkedIdentity[] }>('/account/identities')).identities.filter(
        (identity) => identity.channel === 'matrix'
      ),
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
  unlinkMatrix: (userId: string) =>
    fetchApi<{ unlinked: boolean }>('/account/identities/unlink', {
      method: 'POST',
      body: JSON.stringify({ channel: 'matrix', user_id: userId }),
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
  revokeSession: (id: string) =>
    fetchApi<boolean>(`/account/sessions/${encodeURIComponent(id)}`, { method: 'DELETE' }),
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
    identity_linked: 'Chat identity linked',
    identity_unlinked: 'Chat identity unlinked',
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
