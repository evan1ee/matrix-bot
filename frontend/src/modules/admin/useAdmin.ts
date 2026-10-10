import { useQuery } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import { useSensitiveAction } from '../../core/security';
import {
  ConfigStatus,
  SettingItem,
  SecretMetadata,
  SecretTestResult,
  AuditLogPage,
} from './types';

export const ADMIN_KEYS = {
  status: ['admin', 'status'] as const,
  settings: ['admin', 'settings'] as const,
  secrets: ['admin', 'secrets'] as const,
  audit: (limit: number, offset: number) => ['admin', 'audit', limit, offset] as const,
};

export function useAdminStatus() {
  return useQuery<ConfigStatus>({
    queryKey: ADMIN_KEYS.status,
    queryFn: () => fetchApi<ConfigStatus>('/admin/config-status'),
    staleTime: 10_000,
    retry: false,
  });
}

export function useAdminSettings() {
  return useQuery<SettingItem[]>({
    queryKey: ADMIN_KEYS.settings,
    queryFn: () => fetchApi<SettingItem[]>('/admin/settings'),
    staleTime: 15_000,
    retry: false,
  });
}

export function useUpdateSetting() {
  return useSensitiveAction(
    ({ key, value }: { key: string; value: any }) =>
      fetchApi<SettingItem>(`/admin/settings/${encodeURIComponent(key)}`, {
        method: 'PUT',
        body: JSON.stringify({ value }),
      }),
    [ADMIN_KEYS.settings, ADMIN_KEYS.status, ['admin', 'audit']]
  );
}

export function useResetSetting() {
  return useSensitiveAction(
    (key: string) =>
      fetchApi<{ status: string }>(`/admin/settings/${encodeURIComponent(key)}`, {
        method: 'DELETE',
      }),
    [ADMIN_KEYS.settings, ADMIN_KEYS.status, ['admin', 'audit']]
  );
}

export function useAdminSecrets() {
  return useQuery<SecretMetadata[]>({
    queryKey: ADMIN_KEYS.secrets,
    queryFn: () => fetchApi<SecretMetadata[]>('/admin/secrets'),
    staleTime: 15_000,
    retry: false,
  });
}

export function useCreateSecret() {
  return useSensitiveAction(
    (payload: {
      provider: string;
      name?: string;
      secret: string;
      metadata?: Record<string, any>;
    }) =>
      fetchApi<SecretMetadata>('/admin/secrets', {
        method: 'POST',
        body: JSON.stringify(payload),
      }),
    [ADMIN_KEYS.secrets, ADMIN_KEYS.status, ['admin', 'audit']]
  );
}

export function useReplaceSecret() {
  return useSensitiveAction(
    ({
      id,
      secret,
      status,
    }: {
      id: string;
      secret?: string;
      status?: string;
    }) =>
      fetchApi<SecretMetadata>(`/admin/secrets/${id}`, {
        method: 'PUT',
        body: JSON.stringify({ secret, status }),
      }),
    [ADMIN_KEYS.secrets, ADMIN_KEYS.status, ['admin', 'audit']]
  );
}

export function useDeleteSecret() {
  return useSensitiveAction(
    (id: string) =>
      fetchApi<{ status: string }>(`/admin/secrets/${id}`, {
        method: 'DELETE',
      }),
    [ADMIN_KEYS.secrets, ADMIN_KEYS.status, ['admin', 'audit']]
  );
}

export function useTestSecret() {
  return useSensitiveAction(
    (id: string) =>
      fetchApi<SecretTestResult>(`/admin/secrets/${id}/test`, {
        method: 'POST',
      }),
    [ADMIN_KEYS.secrets, ADMIN_KEYS.status, ['admin', 'audit']]
  );
}

export function useAdminAudit(limit = 50, offset = 0) {
  return useQuery<AuditLogPage>({
    queryKey: ADMIN_KEYS.audit(limit, offset),
    queryFn: () => fetchApi<AuditLogPage>(`/admin/audit-logs?limit=${limit}&offset=${offset}`),
    staleTime: 10_000,
    retry: false,
  });
}
