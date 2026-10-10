import React, { useState } from 'react';
import {
  Shield, Sliders, Cpu, Search, MessageSquare, Key, History,
  Plus, RefreshCw, Trash2, CheckCircle2, AlertTriangle, XCircle
} from 'lucide-react';
import {
  useAdminStatus,
  useAdminSettings,
  useResetSetting,
  useAdminSecrets,
  useDeleteSecret,
  useTestSecret,
  useAdminAudit,
} from './useAdmin';
import { SettingItem, SecretMetadata } from './types';
import { SettingsTable } from './SettingsTable';
import { EditSettingModal } from './EditSettingModal';
import { SecretModal } from './SecretModal';
import { ConfirmModal } from '../../components/ConfirmModal';

type TabType = 'overview' | 'settings' | 'llm' | 'inference' | 'channels' | 'secrets' | 'audit';

export const AdminView: React.FC = () => {
  const [activeTab, setActiveTab] = useState<TabType>('overview');

  const { data: status, isLoading: statusLoading } = useAdminStatus();
  const { data: settings = [], isLoading: settingsLoading } = useAdminSettings();
  const { data: secrets = [], isLoading: secretsLoading } = useAdminSecrets();
  const { data: auditLogs = [], isLoading: auditLoading } = useAdminAudit(50, 0);

  const resetSettingMutation = useResetSetting();
  const deleteSecretMutation = useDeleteSecret();
  const testSecretMutation = useTestSecret();

  const [editingSetting, setEditingSetting] = useState<SettingItem | null>(null);
  const [isSecretModalOpen, setIsSecretModalOpen] = useState(false);
  const [editingSecret, setEditingSecret] = useState<SecretMetadata | null>(null);

  const [settingToReset, setSettingToReset] = useState<SettingItem | null>(null);
  const [secretToDelete, setSecretToDelete] = useState<SecretMetadata | null>(null);
  const [testFeedback, setTestFeedback] = useState<{ id: string; status: string; error?: string | null } | null>(null);

  const handleTestSecret = async (secretId: string) => {
    try {
      const res = await testSecretMutation.mutateAsync(secretId);
      setTestFeedback({ id: secretId, status: res.status, error: res.error });
    } catch (err: any) {
      setTestFeedback({ id: secretId, status: 'failed', error: err?.message || 'Test request failed' });
    }
  };

  const filterSettingsByCategory = (category: string) =>
    settings.filter((s) => s.category.toLowerCase() === category.toLowerCase());

  return (
    <div className="space-y-6">
      {/* Module Header */}
      <div className="flex flex-col gap-1 sm:flex-row sm:items-center sm:justify-between border-b border-[var(--color-border-subtle)] pb-4">
        <div>
          <div className="flex items-center gap-2">
            <Shield className="h-5 w-5 text-neutral-900 dark:text-neutral-100" />
            <h1 className="text-xl font-semibold tracking-tight text-[var(--color-text-primary)]">
              Admin Console
            </h1>
          </div>
          <p className="text-xs text-[var(--color-text-secondary)] mt-1">
            System configuration, encrypted credentials vault, and dynamic runtime management
          </p>
        </div>

        {status && (
          <div className="flex items-center gap-3 text-xs">
            <span
              className={`inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 font-medium ${
                status.encryption_configured
                  ? 'bg-emerald-500/10 text-emerald-600 dark:text-emerald-400'
                  : 'bg-amber-500/10 text-amber-600 dark:text-amber-400'
              }`}
            >
              <span className={`h-1.5 w-1.5 rounded-full ${status.encryption_configured ? 'bg-emerald-500' : 'bg-amber-500'}`} />
              {status.encryption_configured ? 'Encryption Active' : 'Encryption Not Configured'}
            </span>
          </div>
        )}
      </div>

      {/* Tabs */}
      <div className="flex overflow-x-auto border-b border-[var(--color-border-subtle)] gap-1">
        {[
          { id: 'overview', label: 'Overview', icon: Shield },
          { id: 'settings', label: 'General', icon: Sliders },
          { id: 'llm', label: 'LLM & Models', icon: Cpu },
          { id: 'inference', label: 'Search & Inference', icon: Search },
          { id: 'channels', label: 'Channels', icon: MessageSquare },
          { id: 'secrets', label: 'Secrets Vault', icon: Key },
          { id: 'audit', label: 'Audit Log', icon: History },
        ].map((tab) => {
          const Icon = tab.icon;
          const isActive = activeTab === tab.id;
          return (
            <button
              key={tab.id}
              onClick={() => setActiveTab(tab.id as TabType)}
              className={`flex items-center gap-2 px-3 py-2 text-xs font-medium border-b-2 transition-colors cursor-pointer whitespace-nowrap ${
                isActive
                  ? 'border-neutral-900 text-neutral-900 dark:border-white dark:text-white'
                  : 'border-transparent text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]'
              }`}
            >
              <Icon className="h-3.5 w-3.5" />
              {tab.label}
            </button>
          );
        })}
      </div>

      {/* ─── TAB 1: OVERVIEW ─── */}
      {activeTab === 'overview' && (
        <div className="space-y-6">
          {statusLoading ? (
            <div className="text-xs text-[var(--color-text-tertiary)]">Loading system status...</div>
          ) : status ? (
            <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-4">
              <div className="rounded-lg border border-[var(--color-border-subtle)] bg-[var(--color-surface)] p-4 shadow-2xs">
                <div className="text-xs text-[var(--color-text-secondary)]">Runtime Settings</div>
                <div className="mt-2 text-2xl font-semibold text-[var(--color-text-primary)]">
                  {status.settings_count}
                </div>
                <div className="mt-1 text-[11px] text-[var(--color-text-tertiary)]">
                  {status.custom_settings_count} customized in database
                </div>
              </div>

              <div className="rounded-lg border border-[var(--color-border-subtle)] bg-[var(--color-surface)] p-4 shadow-2xs">
                <div className="text-xs text-[var(--color-text-secondary)]">Encrypted Credentials</div>
                <div className="mt-2 text-2xl font-semibold text-[var(--color-text-primary)]">
                  {status.secrets_count}
                </div>
                <div className="mt-1 text-[11px] text-[var(--color-text-tertiary)]">
                  {status.active_secrets_count} active in vault
                </div>
              </div>

              <div className="rounded-lg border border-[var(--color-border-subtle)] bg-[var(--color-surface)] p-4 shadow-2xs">
                <div className="text-xs text-[var(--color-text-secondary)]">Cryptographic Security</div>
                <div className="mt-2 text-lg font-semibold text-[var(--color-text-primary)] flex items-center gap-1.5">
                  {status.encryption_configured ? (
                    <>
                      <CheckCircle2 className="h-4 w-4 text-emerald-500" />
                      <span>AES-256-GCM</span>
                    </>
                  ) : (
                    <>
                      <AlertTriangle className="h-4 w-4 text-amber-500" />
                      <span>Fails Closed</span>
                    </>
                  )}
                </div>
                <div className="mt-1 text-[11px] text-[var(--color-text-tertiary)]">
                  {status.encryption_configured ? 'Hardware-grade AAD binding' : 'SECRETS_ENCRYPTION_KEY missing'}
                </div>
              </div>

              <div className="rounded-lg border border-[var(--color-border-subtle)] bg-[var(--color-surface)] p-4 shadow-2xs">
                <div className="text-xs text-[var(--color-text-secondary)]">Dynamic Snapshot</div>
                <div className="mt-2 text-xs font-mono text-[var(--color-text-primary)] truncate">
                  {new Date(status.last_reload_at).toLocaleTimeString()}
                </div>
                <div className="mt-1 text-[11px] text-[var(--color-text-tertiary)]">
                  Lock-free ArcSwap active
                </div>
              </div>
            </div>
          ) : null}

          {/* Restart Required Warning Card */}
          <div className="rounded-lg border border-amber-500/20 bg-amber-500/5 p-4 text-xs">
            <div className="flex items-center gap-2 font-medium text-amber-600 dark:text-amber-400">
              <AlertTriangle className="h-4 w-4" />
              <span>Restart & Hot-Reload Notices</span>
            </div>
            <p className="mt-1 text-[var(--color-text-secondary)] leading-relaxed">
              Settings with <span className="font-mono text-[10px] bg-neutral-200 dark:bg-neutral-800 px-1 py-0.5 rounded">hot_reloadable: false</span> (such as channel credentials and knowledge embedding models) will take effect after restarting the service process. LLM models, prompts, temperature, and search settings reload immediately on the next request.
            </p>
          </div>
        </div>
      )}

      {/* ─── TAB 2: GENERAL SETTINGS ─── */}
      {activeTab === 'settings' && (
        <SettingsTable
          settings={filterSettingsByCategory('General').concat(filterSettingsByCategory('Registration'))}
          onEdit={setEditingSetting}
          onReset={setSettingToReset}
        />
      )}

      {/* ─── TAB 3: LLM & MODELS ─── */}
      {activeTab === 'llm' && (
        <SettingsTable
          settings={filterSettingsByCategory('LLM')}
          onEdit={setEditingSetting}
          onReset={setSettingToReset}
        />
      )}

      {/* ─── TAB 4: SEARCH & INFERENCE ─── */}
      {activeTab === 'inference' && (
        <SettingsTable
          settings={filterSettingsByCategory('Search').concat(filterSettingsByCategory('Inference'))}
          onEdit={setEditingSetting}
          onReset={setSettingToReset}
        />
      )}

      {/* ─── TAB 5: CHANNELS ─── */}
      {activeTab === 'channels' && (
        <SettingsTable
          settings={filterSettingsByCategory('Channels')}
          onEdit={setEditingSetting}
          onReset={setSettingToReset}
        />
      )}

      {/* ─── TAB 6: SECRETS VAULT ─── */}
      {activeTab === 'secrets' && (
        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <p className="text-xs text-[var(--color-text-secondary)]">
              Encrypted credentials stored with AES-256-GCM. Plaintext is write-only and never exposed in responses or logs.
            </p>
            <button
              onClick={() => {
                setEditingSecret(null);
                setIsSecretModalOpen(true);
              }}
              className="flex items-center gap-1.5 rounded-md bg-neutral-900 px-3 py-1.5 text-xs font-medium text-white hover:bg-neutral-800 dark:bg-white dark:text-neutral-900 dark:hover:bg-neutral-200 cursor-pointer"
            >
              <Plus className="h-3.5 w-3.5" />
              <span>Add Credential</span>
            </button>
          </div>

          {secretsLoading ? (
            <div className="text-xs text-[var(--color-text-tertiary)]">Loading encrypted vault...</div>
          ) : secrets.length === 0 ? (
            <div className="rounded-lg border border-dashed border-[var(--color-border-subtle)] p-8 text-center text-xs text-[var(--color-text-secondary)]">
              No external credentials currently configured. Click &quot;Add Credential&quot; to configure one.
            </div>
          ) : (
            <div className="overflow-x-auto rounded-lg border border-[var(--color-border-subtle)]">
              <table className="w-full text-left text-xs">
                <thead className="bg-[var(--color-surface-hover)] border-b border-[var(--color-border-subtle)] font-medium text-[var(--color-text-secondary)]">
                  <tr>
                    <th className="px-4 py-2.5">Provider</th>
                    <th className="px-4 py-2.5">Slot Name</th>
                    <th className="px-4 py-2.5">Masked Preview</th>
                    <th className="px-4 py-2.5">Version</th>
                    <th className="px-4 py-2.5">Status</th>
                    <th className="px-4 py-2.5">Connectivity</th>
                    <th className="px-4 py-2.5 text-right">Actions</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-[var(--color-border-subtle)]">
                  {secrets.map((sec) => (
                    <tr key={sec.id} className="hover:bg-[var(--color-surface-hover)] transition-colors">
                      <td className="px-4 py-3 font-semibold text-[var(--color-text-primary)] capitalize">
                        {sec.provider}
                      </td>
                      <td className="px-4 py-3 font-mono text-[var(--color-text-secondary)]">
                        {sec.name}
                      </td>
                      <td className="px-4 py-3 font-mono text-[var(--color-text-secondary)]">
                        {sec.masked_preview || '••••••••'}
                      </td>
                      <td className="px-4 py-3 font-mono text-[var(--color-text-tertiary)]">
                        v{sec.key_version}
                      </td>
                      <td className="px-4 py-3">
                        <span className="inline-flex rounded-full px-2 py-0.5 text-[10px] font-medium bg-neutral-100 text-neutral-700 dark:bg-neutral-800 dark:text-neutral-300">
                          {sec.status}
                        </span>
                      </td>
                      <td className="px-4 py-3">
                        {testFeedback?.id === sec.id ? (
                          <span className={`inline-flex items-center gap-1 text-[11px] ${
                            testFeedback.status === 'healthy' ? 'text-emerald-600 dark:text-emerald-400' : 'text-red-600 dark:text-red-400'
                          }`}>
                            {testFeedback.status === 'healthy' ? <CheckCircle2 className="h-3 w-3" /> : <XCircle className="h-3 w-3" />}
                            {testFeedback.status}
                          </span>
                        ) : sec.last_test_status ? (
                          <span className={`inline-flex items-center gap-1 text-[11px] ${
                            sec.last_test_status === 'healthy' ? 'text-emerald-600 dark:text-emerald-400' : 'text-amber-600 dark:text-amber-400'
                          }`}>
                            {sec.last_test_status === 'healthy' ? <CheckCircle2 className="h-3 w-3" /> : <AlertTriangle className="h-3 w-3" />}
                            {sec.last_test_status}
                          </span>
                        ) : (
                          <span className="text-[var(--color-text-tertiary)] text-[11px]">Untested</span>
                        )}
                      </td>
                      <td className="px-4 py-3 text-right">
                        <div className="flex items-center justify-end gap-1.5">
                          <button
                            onClick={() => handleTestSecret(sec.id)}
                            title="Test Connectivity"
                            className="rounded p-1 text-[var(--color-text-secondary)] hover:bg-[var(--color-surface-hover)] hover:text-[var(--color-text-primary)] cursor-pointer"
                          >
                            <RefreshCw className="h-3.5 w-3.5" />
                          </button>
                          <button
                            onClick={() => {
                              setEditingSecret(sec);
                              setIsSecretModalOpen(true);
                            }}
                            title="Replace Secret"
                            className="rounded px-2 py-1 text-xs text-[var(--color-text-secondary)] hover:bg-[var(--color-surface-hover)] hover:text-[var(--color-text-primary)] cursor-pointer"
                          >
                            Replace
                          </button>
                          <button
                            onClick={() => setSecretToDelete(sec)}
                            title="Delete Credential"
                            className="rounded p-1 text-red-500 hover:bg-red-500/10 cursor-pointer"
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                          </button>
                        </div>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      )}

      {/* ─── TAB 7: AUDIT LOG ─── */}
      {activeTab === 'audit' && (
        <div className="space-y-4">
          <p className="text-xs text-[var(--color-text-secondary)]">
            Append-only audit record of all administrative settings and encrypted secret modifications.
          </p>

          {auditLoading ? (
            <div className="text-xs text-[var(--color-text-tertiary)]">Loading audit records...</div>
          ) : auditLogs.length === 0 ? (
            <div className="rounded-lg border border-dashed border-[var(--color-border-subtle)] p-8 text-center text-xs text-[var(--color-text-secondary)]">
              No administrative configuration changes recorded yet.
            </div>
          ) : (
            <div className="overflow-x-auto rounded-lg border border-[var(--color-border-subtle)]">
              <table className="w-full text-left text-xs">
                <thead className="bg-[var(--color-surface-hover)] border-b border-[var(--color-border-subtle)] font-medium text-[var(--color-text-secondary)]">
                  <tr>
                    <th className="px-4 py-2.5">Timestamp</th>
                    <th className="px-4 py-2.5">Action</th>
                    <th className="px-4 py-2.5">Target</th>
                    <th className="px-4 py-2.5">Outcome</th>
                    <th className="px-4 py-2.5">Details</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-[var(--color-border-subtle)]">
                  {auditLogs.map((entry) => (
                    <tr key={entry.id} className="hover:bg-[var(--color-surface-hover)] transition-colors">
                      <td className="px-4 py-3 font-mono text-[11px] text-[var(--color-text-secondary)]">
                        {new Date(entry.created_at).toLocaleString()}
                      </td>
                      <td className="px-4 py-3 font-medium text-[var(--color-text-primary)]">
                        {entry.action}
                      </td>
                      <td className="px-4 py-3 font-mono text-[var(--color-text-secondary)]">
                        {entry.target_key}
                      </td>
                      <td className="px-4 py-3">
                        <span className={`inline-flex rounded-full px-2 py-0.5 text-[10px] font-medium ${
                          entry.outcome === 'success'
                            ? 'bg-emerald-500/10 text-emerald-600 dark:text-emerald-400'
                            : 'bg-red-500/10 text-red-600 dark:text-red-400'
                        }`}>
                          {entry.outcome}
                        </span>
                      </td>
                      <td className="px-4 py-3 font-mono text-[11px] text-[var(--color-text-tertiary)] truncate max-w-xs">
                        {entry.detail ? JSON.stringify(entry.detail) : '—'}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      )}

      {/* Modals */}
      <EditSettingModal
        setting={editingSetting}
        onClose={() => setEditingSetting(null)}
      />

      <SecretModal
        isOpen={isSecretModalOpen}
        editingSecret={editingSecret}
        onClose={() => {
          setIsSecretModalOpen(false);
          setEditingSecret(null);
        }}
      />

      <ConfirmModal
        isOpen={!!settingToReset}
        title="Reset Setting to Default"
        message={`Are you sure you want to reset "${settingToReset?.key}" to its built-in default value?`}
        confirmLabel="Reset"
        onConfirm={async () => {
          if (settingToReset) {
            await resetSettingMutation.mutateAsync(settingToReset.key);
            setSettingToReset(null);
          }
        }}
        onClose={() => setSettingToReset(null)}
      />

      <ConfirmModal
        isOpen={!!secretToDelete}
        title="Delete Encrypted Credential"
        message={`Are you sure you want to delete credential slot "${secretToDelete?.provider}:${secretToDelete?.name}"? External features relying on this credential will cease to operate.`}
        confirmLabel="Delete"
        isDestructive
        onConfirm={async () => {
          if (secretToDelete) {
            await deleteSecretMutation.mutateAsync(secretToDelete.id);
            setSecretToDelete(null);
          }
        }}
        onClose={() => setSecretToDelete(null)}
      />
    </div>
  );
};
