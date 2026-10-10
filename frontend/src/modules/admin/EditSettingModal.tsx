import React, { useState, useEffect } from 'react';
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

  if (!setting) return null;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    let parsedVal: any = valStr;
    if (setting.value_type === 'boolean') {
      parsedVal = valStr === 'true';
    } else if (setting.value_type === 'integer') {
      const parsed = parseInt(valStr, 10);
      if (isNaN(parsed)) {
        setError('Please enter a valid integer');
        return;
      }
      parsedVal = parsed;
    } else if (setting.value_type === 'float') {
      const parsed = parseFloat(valStr);
      if (isNaN(parsed)) {
        setError('Please enter a valid float number');
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
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4 backdrop-blur-xs"
      onClick={onClose}
    >
      <div
        className="w-full max-w-lg rounded-xl border border-[var(--color-border-subtle)] bg-[var(--color-surface)] p-6 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between pb-3 border-b border-[var(--color-border-subtle)]">
          <div>
            <h3 className="text-base font-semibold text-[var(--color-text-primary)]">
              Edit Setting
            </h3>
            <p className="font-mono text-xs text-[var(--color-text-secondary)] mt-0.5">
              {setting.key}
            </p>
          </div>
          <button
            onClick={onClose}
            className="rounded p-1 text-[var(--color-text-tertiary)] hover:text-[var(--color-text-primary)]"
          >
            ✕
          </button>
        </div>

        <p className="mt-3 text-xs text-[var(--color-text-secondary)]">
          {setting.description}
        </p>

        {error && (
          <div className="mt-3 rounded-md bg-red-500/10 p-2.5 text-xs text-red-600 dark:text-red-400">
            {error}
          </div>
        )}

        <form onSubmit={handleSubmit} className="mt-4 space-y-4">
          <div>
            <label className="block text-xs font-medium text-[var(--color-text-secondary)] mb-1">
              Value ({setting.value_type})
            </label>
            {setting.value_type === 'boolean' ? (
              <select
                value={valStr}
                onChange={(e) => setValStr(e.target.value)}
                className="w-full rounded-md border border-[var(--color-border-subtle)] bg-[var(--color-surface-hover)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus:outline-hidden focus:ring-1 focus:ring-[var(--color-border-strong)]"
              >
                <option value="true">true</option>
                <option value="false">false</option>
              </select>
            ) : setting.value_type === 'json' ? (
              <textarea
                rows={5}
                value={valStr}
                onChange={(e) => setValStr(e.target.value)}
                className="w-full font-mono rounded-md border border-[var(--color-border-subtle)] bg-[var(--color-surface-hover)] p-3 text-xs text-[var(--color-text-primary)] focus:outline-hidden focus:ring-1 focus:ring-[var(--color-border-strong)]"
              />
            ) : (
              <input
                type={setting.value_type === 'integer' || setting.value_type === 'float' ? 'number' : 'text'}
                step={setting.value_type === 'float' ? '0.05' : undefined}
                value={valStr}
                onChange={(e) => setValStr(e.target.value)}
                className="w-full rounded-md border border-[var(--color-border-subtle)] bg-[var(--color-surface-hover)] px-3 py-2 text-sm text-[var(--color-text-primary)] focus:outline-hidden focus:ring-1 focus:ring-[var(--color-border-strong)]"
              />
            )}
          </div>

          <div className="flex items-center justify-end gap-2 pt-2 border-t border-[var(--color-border-subtle)]">
            <button
              type="button"
              onClick={onClose}
              className="rounded-md px-3 py-1.5 text-xs text-[var(--color-text-secondary)] hover:bg-[var(--color-surface-hover)] cursor-pointer"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={updateMutation.isPending}
              className="rounded-md bg-neutral-900 px-4 py-1.5 text-xs font-medium text-white hover:bg-neutral-800 dark:bg-white dark:text-neutral-900 dark:hover:bg-neutral-200 cursor-pointer disabled:opacity-50"
            >
              {updateMutation.isPending ? 'Saving...' : 'Save Setting'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
