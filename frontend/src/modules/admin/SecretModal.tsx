import React, { useState, useEffect } from 'react';
import { SecretMetadata } from './types';
import { useCreateSecret, useReplaceSecret } from './useAdmin';

interface SecretModalProps {
  isOpen: boolean;
  editingSecret: SecretMetadata | null;
  onClose: () => void;
}

const PROVIDERS = [
  { value: 'openai', label: 'OpenAI' },
  { value: 'anthropic', label: 'Anthropic' },
  { value: 'tavily', label: 'Tavily Search' },
  { value: 'telegram', label: 'Telegram Bot' },
  { value: 'discord', label: 'Discord Bot' },
  { value: 'email', label: 'Email (IMAP/SMTP)' },
  { value: 'stt', label: 'Speech-to-Text' },
  { value: 'tts', label: 'Text-to-Speech' },
  { value: 's3', label: 'S3 Storage' },
  { value: 'generic', label: 'Generic' },
];

export const SecretModal: React.FC<SecretModalProps> = ({
  isOpen,
  editingSecret,
  onClose,
}) => {
  const [provider, setProvider] = useState('openai');
  const [name, setName] = useState('default');
  const [secretValue, setSecretValue] = useState('');
  const [error, setError] = useState<string | null>(null);

  const createMutation = useCreateSecret();
  const replaceMutation = useReplaceSecret();

  useEffect(() => {
    // Always scrub sensitive state when opening/closing/switching
    setSecretValue('');
    setError(null);
    if (editingSecret) {
      setProvider(editingSecret.provider);
      setName(editingSecret.name);
    } else {
      setProvider('openai');
      setName('default');
    }
  }, [isOpen, editingSecret]);

  if (!isOpen) return null;

  const handleClose = () => {
    setSecretValue('');
    setError(null);
    onClose();
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    if (!secretValue.trim()) {
      setError('Secret value cannot be empty');
      return;
    }

    try {
      if (editingSecret) {
        await replaceMutation.mutateAsync({
          id: editingSecret.id,
          secret: secretValue.trim(),
        });
      } else {
        await createMutation.mutateAsync({
          provider,
          name: name.trim() || 'default',
          secret: secretValue.trim(),
        });
      }
      // Scrub sensitive input immediately after submission
      setSecretValue('');
      onClose();
    } catch (err: any) {
      setError(err?.message || 'Failed to save secret');
    }
  };

  const isPending = createMutation.isPending || replaceMutation.isPending;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4 backdrop-blur-xs"
      onClick={handleClose}
    >
      <div
        className="w-full max-w-md rounded-xl border border-[var(--color-border-subtle)] bg-[var(--color-surface)] p-6 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between pb-3 border-b border-[var(--color-border-subtle)]">
          <h3 className="text-base font-semibold text-[var(--color-text-primary)]">
            {editingSecret ? 'Replace Encrypted Credential' : 'Add Encrypted Credential'}
          </h3>
          <button
            onClick={handleClose}
            className="rounded p-1 text-[var(--color-text-tertiary)] hover:text-[var(--color-text-primary)]"
          >
            ✕
          </button>
        </div>

        {error && (
          <div className="mt-3 rounded-md bg-red-500/10 p-2.5 text-xs text-red-600 dark:text-red-400">
            {error}
          </div>
        )}

        <form onSubmit={handleSubmit} className="mt-4 space-y-4">
          <div>
            <label className="block text-xs font-medium text-[var(--color-text-secondary)] mb-1">
              Provider
            </label>
            <select
              disabled={!!editingSecret}
              value={provider}
              onChange={(e) => setProvider(e.target.value)}
              aria-label="Provider"
              className="w-full rounded-md border border-[var(--color-border-subtle)] bg-[var(--color-surface-hover)] px-3 py-2 text-sm text-[var(--color-text-primary)] disabled:opacity-60 focus:outline-hidden focus:ring-1 focus:ring-[var(--color-border-strong)]"
            >
              {PROVIDERS.map((p) => (
                <option key={p.value} value={p.value}>
                  {p.label}
                </option>
              ))}
            </select>
          </div>

          <div>
            <label className="block text-xs font-medium text-[var(--color-text-secondary)] mb-1">
              Credential Slot Name
            </label>
            <input
              type="text"
              disabled={!!editingSecret}
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="default"
              aria-label="Credential Slot Name"
              className="w-full font-mono rounded-md border border-[var(--color-border-subtle)] bg-[var(--color-surface-hover)] px-3 py-2 text-sm text-[var(--color-text-primary)] disabled:opacity-60 focus:outline-hidden focus:ring-1 focus:ring-[var(--color-border-strong)]"
            />
          </div>

          <div>
            <label className="block text-xs font-medium text-[var(--color-text-secondary)] mb-1">
              Secret Value (Write-Only)
            </label>
            <input
              type="password"
              autoComplete="new-password"
              value={secretValue}
              onChange={(e) => setSecretValue(e.target.value)}
              placeholder={
                editingSecret
                  ? `${editingSecret.masked_preview || '••••••••'} (Enter new secret to replace)`
                  : 'Enter API key or token'
              }
              aria-label="Secret Value"
              className="w-full font-mono rounded-md border border-[var(--color-border-subtle)] bg-[var(--color-surface-hover)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus:outline-hidden focus:ring-1 focus:ring-[var(--color-border-strong)]"
            />
            <p className="mt-1 text-[11px] text-[var(--color-text-tertiary)]">
              Encrypted with AES-256-GCM at rest. Never returned in API responses.
            </p>
          </div>

          <div className="flex items-center justify-end gap-2 pt-2 border-t border-[var(--color-border-subtle)]">
            <button
              type="button"
              onClick={handleClose}
              className="rounded-md px-3 py-1.5 text-xs text-[var(--color-text-secondary)] hover:bg-[var(--color-surface-hover)] cursor-pointer"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={isPending}
              className="rounded-md bg-neutral-900 px-4 py-1.5 text-xs font-medium text-white hover:bg-neutral-800 dark:bg-white dark:text-neutral-900 dark:hover:bg-neutral-200 cursor-pointer disabled:opacity-50"
            >
              {isPending ? 'Encrypting...' : editingSecret ? 'Replace Secret' : 'Store Secret'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
