export interface SettingItem {
  key: string;
  value: any;
  default_value: any;
  value_type: 'string' | 'integer' | 'float' | 'boolean' | 'json';
  description: string;
  is_custom: boolean;
  hot_reloadable: boolean;
  category: string;
  updated_at?: string | null;
  updated_by?: string | null;
}

export interface SecretMetadata {
  id: string;
  provider: string;
  name: string;
  status: 'active' | 'disabled' | 'revoked';
  key_version: number;
  masked_preview?: string | null;
  last_tested_at?: string | null;
  last_test_status?: 'healthy' | 'unhealthy' | 'failed' | 'unknown' | null;
  last_test_error?: string | null;
  created_at: string;
  updated_at: string;
}

export interface SecretTestResult {
  id: string;
  status: string;
  error?: string | null;
}

export interface ConfigStatus {
  settings_count: number;
  custom_settings_count: number;
  encryption_configured: boolean;
  secrets_count: number;
  active_secrets_count: number;
  last_reload_at: string;
}

export interface AuditLogEntry {
  id: number;
  actor_id: string | null;
  action: string;
  target_type: string;
  target_key: string;
  outcome: string;
  detail: any;
  created_at: string;
}

export interface AuditLogPage {
  entries: AuditLogEntry[];
  total: number;
  limit: number;
  offset: number;
}
