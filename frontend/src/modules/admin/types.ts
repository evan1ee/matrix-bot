import type { components } from '../../types/api';

type Schemas = components['schemas'];

// Aliases of the generated DTOs: a backend change that breaks the admin
// console now fails `npm run build`, not the browser.
export type SettingItem = Schemas['SettingItemDto'];
export type SecretMetadata = Schemas['SecretMetadataDto'];
export type SecretTestResult = Schemas['SecretTestResultDto'];
export type ConfigStatus = Schemas['ConfigurationStatusDto'];
export type AuditLogEntry = Schemas['ConfigAuditRowDto'];
export type AuditLogPage = Schemas['AuditLogPageDto'];
