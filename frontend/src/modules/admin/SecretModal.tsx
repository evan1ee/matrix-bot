import React, { useState, useEffect, useCallback } from 'react';
import { X } from 'lucide-react';
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
  { value: 'stt', label: 'Speech-to-Text' },
  { value: 'tts', label: 'Text-to-Speech' },
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
  const isPending = createMutation.isPending || replaceMutation.isPending;

  const handleClose = useCallback(() => {
    setSecretValue('');
    setError(null);
    createMutation.reset();
    replaceMutation.reset();
    onClose();
  }, [createMutation, replaceMutation, onClose]);

  useEffect(() => {
    // Always scrub sensitive state when opening/closing/switching
    setSecretValue('');
    setError(null);
    createMutation.reset();
    replaceMutation.reset();
    if (editingSecret) {
      setProvider(editingSecret.provider);
      setName(editingSecret.name);
    } else {
      setProvider('openai');
      setName('default');
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isOpen, editingSecret]);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !isPending) {
        handleClose();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, isPending, handleClose]);

  if (!isOpen) return null;

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
      // Scrub sensitive input and React Query mutation variables immediately
      setSecretValue('');
      createMutation.reset();
      replaceMutation.reset();
      onClose();
    } catch (err: any) {
      createMutation.reset();
      replaceMutation.reset();
      setError(err?.message || 'Failed to save secret');
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
      onClick={() => {
        if (!isPending) handleClose();
      }}
      role="presentation"
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="secret-modal-title"
        className="w-full max-w-md rounded-lg border border-[var(--color-border-subtle)] bg-[var(--color-surface)] p-6 shadow-sm"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between pb-3 border-b border-[var(--color-border-subtle)]">
          <h3
            id="secret-modal-title"
            className="text-base font-semibold text-[var(--color-text-primary)]"
          >
            {editingSecret ? 'Replace Encrypted Credential' : 'Add Encrypted Credential'}
          </h3>
          <button
            type="button"
            onClick={handleClose}
            aria-label="Close"
            className="rounded p-1 text-[var(--color-text-tertiary)] hover:text-[var(--color-text-primary)] transition-colors duration-150 cursor-pointer"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {error && (
          <div
            role="alert"
            className="mt-3 rounded-md border border-rose-200 dark:border-rose-900/60 bg-rose-50 dark:bg-rose-950/40 p-2.5 text-xs text-rose-600 dark:text-rose-400"
          >
            {error}
          </div>
        )}

        <form onSubmit={handleSubmit} className="mt-4 space-y-4">
          <div>
            <label
              htmlFor="secret-provider-select"
              className="block text-xs font-medium text-[var(--color-text-secondary)] mb-1"
            >
              Provider
            </label>
            <select
              id="secret-provider-select"
              disabled={!!editingSecret}
              value={provider}
              onChange={(e) => setProvider(e.target.value)}
              aria-label="Provider"
              className="w-full rounded-md border border-[var(--color-border-subtle)] bg-[var(--color-surface-hover)] px-3 py-2 text-sm text-[var(--color-text-primary)] disabled:opacity-60 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-neutral-900 dark:focus-visible:outline-neutral-100"
            >
              {PROVIDERS.map((p) => (
                <option key={p.value} value={p.value}>
                  {p.label}
                </option>
              ))}
            </select>
          </div>

          <div>
            <label
              htmlFor="secret-slot-name-input"
              className="block text-xs font-medium text-[var(--color-text-secondary)] mb-1"
            >
              Credential Slot Name
            </label>
            <input
              id="secret-slot-name-input"
              type="text"
              disabled={!!editingSecret}
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="default"
              aria-label="Credential Slot Name"
              className="w-full font-mono rounded-md border border-[var(--color-border-subtle)] bg-[var(--color-surface-hover)] px-3 py-2 text-sm text-[var(--color-text-primary)] disabled:opacity-60 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-neutral-900 dark:focus-visible:outline-neutral-100"
            />
          </div>

          <div>
            <label
              htmlFor="secret-value-input"
              className="block text-xs font-medium text-[var(--color-text-secondary)] mb-1"
            >
              Secret Value (Write-Only)
            </label>
            <input
              id="secret-value-input"
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
              className="w-full font-mono rounded-md border border-[var(--color-border-subtle)] bg-[var(--color-surface-hover)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-neutral-900 dark:focus-visible:outline-neutral-100"
            />
            <p className="mt-1 text-[11px] text-[var(--color-text-tertiary)]">
              Encrypted with AES-256-GCM at rest. Never returned in API responses.
            </p>
          </div>

          <div className="flex items-center justify-end gap-2 pt-2 border-t border-[var(--color-border-subtle)]">
            <button
              type="button"
              onClick={handleClose}
              className="rounded-md px-3 py-1.5 text-xs text-[var(--color-text-secondary)] hover:bg-[var(--color-surface-hover)] transition-colors duration-150 cursor-pointer"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={isPending}
              className="rounded-md bg-neutral-900 px-4 py-1.5 text-xs font-medium text-white hover:bg-neutral-800 dark:bg-white dark:text-neutral-900 dark:hover:bg-neutral-200 transition-colors duration-150 cursor-pointer disabled:opacity-50"
            >
              {isPending ? 'Encrypting...' : editingSecret ? 'Replace Secret' : 'Store Secret'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
