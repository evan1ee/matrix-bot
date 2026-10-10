import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import {
  ConfigStatus,
  SettingItem,
  SecretMetadata,
  SecretTestResult,
  AuditLogEntry,
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
  });
}

export function useAdminSettings() {
  return useQuery<SettingItem[]>({
    queryKey: ADMIN_KEYS.settings,
    queryFn: () => fetchApi<SettingItem[]>('/admin/settings'),
    staleTime: 15_000,
  });
}

export function useUpdateSetting() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ key, value }: { key: string; value: any }) =>
      fetchApi<SettingItem>(`/admin/settings/${encodeURIComponent(key)}`, {
        method: 'PUT',
        body: JSON.stringify({ value }),
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ADMIN_KEYS.settings });
      queryClient.invalidateQueries({ queryKey: ADMIN_KEYS.status });
      queryClient.invalidateQueries({ queryKey: ['admin', 'audit'] });
    },
  });
}

export function useResetSetting() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (key: string) =>
      fetchApi<{ status: string }>(`/admin/settings/${encodeURIComponent(key)}`, {
        method: 'DELETE',
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ADMIN_KEYS.settings });
      queryClient.invalidateQueries({ queryKey: ADMIN_KEYS.status });
      queryClient.invalidateQueries({ queryKey: ['admin', 'audit'] });
    },
  });
}

export function useAdminSecrets() {
  return useQuery<SecretMetadata[]>({
    queryKey: ADMIN_KEYS.secrets,
    queryFn: () => fetchApi<SecretMetadata[]>('/admin/secrets'),
    staleTime: 15_000,
  });
}

export function useCreateSecret() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (payload: {
      provider: string;
      name?: string;
      secret: string;
      metadata?: Record<string, any>;
    }) =>
      fetchApi<SecretMetadata>('/admin/secrets', {
        method: 'POST',
        body: JSON.stringify(payload),
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ADMIN_KEYS.secrets });
      queryClient.invalidateQueries({ queryKey: ADMIN_KEYS.status });
      queryClient.invalidateQueries({ queryKey: ['admin', 'audit'] });
    },
  });
}

export function useReplaceSecret() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
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
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ADMIN_KEYS.secrets });
      queryClient.invalidateQueries({ queryKey: ADMIN_KEYS.status });
      queryClient.invalidateQueries({ queryKey: ['admin', 'audit'] });
    },
  });
}

export function useDeleteSecret() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      fetchApi<{ status: string }>(`/admin/secrets/${id}`, {
        method: 'DELETE',
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ADMIN_KEYS.secrets });
      queryClient.invalidateQueries({ queryKey: ADMIN_KEYS.status });
      queryClient.invalidateQueries({ queryKey: ['admin', 'audit'] });
    },
  });
}

export function useTestSecret() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      fetchApi<SecretTestResult>(`/admin/secrets/${id}/test`, {
        method: 'POST',
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ADMIN_KEYS.secrets });
      queryClient.invalidateQueries({ queryKey: ADMIN_KEYS.status });
      queryClient.invalidateQueries({ queryKey: ['admin', 'audit'] });
    },
  });
}

export function useAdminAudit(limit = 50, offset = 0) {
  return useQuery<AuditLogEntry[]>({
    queryKey: ADMIN_KEYS.audit(limit, offset),
    queryFn: () => fetchApi<AuditLogEntry[]>(`/admin/audit-logs?limit=${limit}&offset=${offset}`),
    staleTime: 10_000,
  });
}
