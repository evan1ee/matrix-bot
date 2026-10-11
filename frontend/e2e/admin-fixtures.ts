import type { components } from '../src/types/api';

type Schemas = components['schemas'];
export type FakeSetting = Schemas['SettingItemDto'];
export type FakeSecret = Schemas['SecretMetadataDto'];
export type FakeAuditRow = Schemas['ConfigAuditRowDto'];

/** The settings a fresh fake backend reports. A new array on every call. */
export function initialAdminSettings(): FakeSetting[] {
  return [
    {
      key: 'llm.default_provider',
      value: 'openai',
      default_value: 'openai',
      value_type: 'string',
      description: 'Default AI provider identifier',
      is_custom: false,
      hot_reloadable: true,
      category: 'LLM',
      updated_at: null,
    },
    {
      key: 'llm.default_model',
      value: 'gpt-4o-mini',
      default_value: 'gpt-4o-mini',
      value_type: 'string',
      description: 'Default model name passed in inference requests',
      is_custom: false,
      hot_reloadable: true,
      category: 'LLM',
      updated_at: null,
    },
    {
      key: 'search.tavily.enabled',
      value: true,
      default_value: true,
      value_type: 'boolean',
      description: 'Enable/disable Tavily live web search',
      is_custom: false,
      hot_reloadable: true,
      category: 'Search',
      updated_at: null,
    },
    {
      key: 'inference.embedding.base_url',
      value: '',
      default_value: '',
      value_type: 'string',
      description: 'Base URL for knowledge embedding inference',
      is_custom: false,
      hot_reloadable: true,
      category: 'inference',
      updated_at: null,
    },
    {
      key: 'registration.allow_link_signup',
      value: true,
      default_value: true,
      value_type: 'boolean',
      description: 'Allow allowlisted chat identities to create accounts via /link',
      is_custom: false,
      hot_reloadable: true,
      category: 'registration',
      updated_at: null,
    },
  ];
}

export function configStatus(
  settings: FakeSetting[],
  secrets: FakeSecret[],
): Schemas['ConfigurationStatusDto'] {
  return {
    settings_count: settings.length,
    custom_settings_count: settings.filter((s) => s.is_custom).length,
    encryption_configured: true,
    secrets_count: secrets.length,
    active_secrets_count: secrets.filter((s) => s.status === 'active').length,
    last_reload_at: new Date().toISOString(),
  };
}

export function auditPage(
  rows: FakeAuditRow[],
  limit: number,
  offset: number,
): Schemas['AuditLogPageDto'] {
  return { entries: rows.slice(offset, offset + limit), total: rows.length, limit, offset };
}
