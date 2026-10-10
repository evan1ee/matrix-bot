import React, { useState, useEffect } from 'react';
import { X } from 'lucide-react';
import { SettingItem } from './types';
import { useUpdateSetting } from './useAdmin';

interface EditSettingModalProps {
  setting: SettingItem | null;
  onClose: () => void;
}

export const EditSettingModal: React.FC<EditSettingModalProps> = ({ setting, onClose }) => {
  const [valStr, setValStr] = useState('');
  const [error, setError] = useState<string | null>(null);
  const updateMutation = useUpdateSetting();

  useEffect(() => {
    if (setting) {
      if (typeof setting.value === 'string') {
        setValStr(setting.value);
      } else {
        setValStr(JSON.stringify(setting.value, null, 2));
      }
      setError(null);
    }
  }, [setting]);

  useEffect(() => {
    if (!setting) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !updateMutation.isPending) {
        onClose();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [setting, updateMutation.isPending, onClose]);

  if (!setting) return null;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    let parsedVal: any = valStr;
    if (setting.value_type === 'boolean') {
      parsedVal = valStr === 'true';
    } else if (setting.value_type === 'integer') {
      const trimmed = valStr.trim();
      if (!/^-?\d+$/.test(trimmed)) {
        setError('Please enter a valid whole integer');
        return;
      }
      const parsed = Number(trimmed);
      if (!Number.isSafeInteger(parsed)) {
        setError('Integer is out of safe range');
        return;
      }
      parsedVal = parsed;
    } else if (setting.value_type === 'float') {
      const trimmed = valStr.trim();
      const parsed = Number(trimmed);
      if (trimmed === '' || !Number.isFinite(parsed)) {
        setError('Please enter a valid number');
        return;
      }
      parsedVal = parsed;
    } else if (setting.value_type === 'json') {
      try {
        parsedVal = JSON.parse(valStr);
      } catch {
        setError('Invalid JSON payload');
        return;
      }
    }

    try {
      await updateMutation.mutateAsync({ key: setting.key, value: parsedVal });
      onClose();
    } catch (err: any) {
      setError(err?.message || 'Failed to update setting');
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
      onClick={() => {
        if (!updateMutation.isPending) onClose();
      }}
      role="presentation"
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="edit-setting-modal-title"
        className="w-full max-w-lg rounded-lg border border-[var(--color-border-subtle)] bg-[var(--color-surface)] p-6 shadow-sm"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between pb-3 border-b border-[var(--color-border-subtle)]">
          <div>
            <h3
              id="edit-setting-modal-title"
              className="text-base font-semibold text-[var(--color-text-primary)]"
            >
              Edit Setting
            </h3>
            <p className="font-mono text-xs text-[var(--color-text-secondary)] mt-0.5">
              {setting.key}
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="rounded p-1 text-[var(--color-text-tertiary)] hover:text-[var(--color-text-primary)] transition-colors duration-150 cursor-pointer"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <p className="mt-3 text-xs text-[var(--color-text-secondary)]">
          {setting.description}
        </p>

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
              htmlFor="setting-value-input"
              className="block text-xs font-medium text-[var(--color-text-secondary)] mb-1"
            >
              Value ({setting.value_type})
            </label>
            {setting.value_type === 'boolean' ? (
              <select
                id="setting-value-input"
                value={valStr}
                onChange={(e) => setValStr(e.target.value)}
                className="w-full rounded-md border border-[var(--color-border-subtle)] bg-[var(--color-surface-hover)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-neutral-900 dark:focus-visible:outline-neutral-100"
              >
                <option value="true">true</option>
                <option value="false">false</option>
              </select>
            ) : setting.value_type === 'json' ? (
              <textarea
                id="setting-value-input"
                rows={5}
                value={valStr}
                onChange={(e) => setValStr(e.target.value)}
                className="w-full font-mono rounded-md border border-[var(--color-border-subtle)] bg-[var(--color-surface-hover)] p-3 text-xs text-[var(--color-text-primary)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-neutral-900 dark:focus-visible:outline-neutral-100"
              />
            ) : (
              <input
                id="setting-value-input"
                type={setting.value_type === 'integer' || setting.value_type === 'float' ? 'number' : 'text'}
                step={setting.value_type === 'float' ? '0.05' : undefined}
                value={valStr}
                onChange={(e) => setValStr(e.target.value)}
                className="w-full rounded-md border border-[var(--color-border-subtle)] bg-[var(--color-surface-hover)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-neutral-900 dark:focus-visible:outline-neutral-100"
              />
            )}
          </div>

          <div className="flex items-center justify-end gap-2 pt-2 border-t border-[var(--color-border-subtle)]">
            <button
              type="button"
              onClick={onClose}
              className="rounded-md px-3 py-1.5 text-xs text-[var(--color-text-secondary)] hover:bg-[var(--color-surface-hover)] transition-colors duration-150 cursor-pointer"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={updateMutation.isPending}
              className="rounded-md bg-neutral-900 px-4 py-1.5 text-xs font-medium text-white hover:bg-neutral-800 dark:bg-white dark:text-neutral-900 dark:hover:bg-neutral-200 transition-colors duration-150 cursor-pointer disabled:opacity-50"
            >
              {updateMutation.isPending ? 'Saving...' : 'Save Setting'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
