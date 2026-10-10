import React from 'react';
import { RotateCcw } from 'lucide-react';
import { SettingItem } from './types';

interface SettingsTableProps {
  settings: SettingItem[];
  onEdit: (s: SettingItem) => void;
  onReset: (s: SettingItem) => void;
}

export const SettingsTable: React.FC<SettingsTableProps> = ({ settings, onEdit, onReset }) => {
  if (settings.length === 0) {
    return (
      <div className="rounded-lg border border-dashed border-[var(--color-border-subtle)] p-8 text-center text-xs text-[var(--color-text-secondary)]">
        No settings in this category.
      </div>
    );
  }

  return (
    <div className="overflow-x-auto rounded-lg border border-[var(--color-border-subtle)]">
      <table className="w-full text-left text-xs">
        <thead className="bg-[var(--color-surface-hover)] border-b border-[var(--color-border-subtle)] font-medium text-[var(--color-text-secondary)]">
          <tr>
            <th className="px-4 py-2.5">Key</th>
            <th className="px-4 py-2.5">Current value</th>
            <th className="px-4 py-2.5">Default</th>
            <th className="px-4 py-2.5">Reload</th>
            <th className="px-4 py-2.5">Description</th>
            <th className="px-4 py-2.5 text-right">Actions</th>
          </tr>
        </thead>
        <tbody className="divide-y divide-[var(--color-border-subtle)]">
          {settings.map((s) => (
            <tr key={s.key} className="hover:bg-[var(--color-surface-hover)] transition-colors duration-150">
              <td className="px-4 py-3 font-mono font-medium text-[var(--color-text-primary)]">
                {s.key}
              </td>
              <td className="px-4 py-3 font-mono">
                <span className={s.is_custom ? 'font-semibold text-[var(--color-text-primary)]' : 'text-[var(--color-text-secondary)]'}>
                  {typeof s.value === 'object' ? JSON.stringify(s.value) : String(s.value)}
                </span>
                {s.is_custom && (
                  <span className="ml-1.5 inline-block text-[10px] rounded border border-neutral-300 dark:border-neutral-700 bg-neutral-100 dark:bg-neutral-800 px-1.5 py-0.5 text-neutral-700 dark:text-neutral-300">
                    custom
                  </span>
                )}
              </td>
              <td className="px-4 py-3 font-mono text-[var(--color-text-tertiary)] text-[11px]">
                {typeof s.default_value === 'object' ? JSON.stringify(s.default_value) : String(s.default_value)}
              </td>
              <td className="px-4 py-3">
                <span className="inline-flex rounded border border-neutral-200 dark:border-neutral-800 bg-neutral-50 dark:bg-neutral-900 px-2 py-0.5 text-[10px] font-mono text-[var(--color-text-secondary)]">
                  {s.hot_reloadable ? 'instant' : 'restart'}
                </span>
              </td>
              <td className="px-4 py-3 text-[var(--color-text-secondary)] max-w-xs truncate">
                {s.description}
              </td>
              <td className="px-4 py-3 text-right">
                <div className="flex items-center justify-end gap-1.5">
                  <button
                    type="button"
                    onClick={() => onEdit(s)}
                    className="rounded px-2 py-1 text-xs text-[var(--color-text-secondary)] hover:bg-[var(--color-surface-hover)] hover:text-[var(--color-text-primary)] transition-colors duration-150 cursor-pointer"
                  >
                    Edit
                  </button>
                  {s.is_custom && (
                    <button
                      type="button"
                      onClick={() => onReset(s)}
                      title="Reset to default"
                      aria-label={`Reset ${s.key} to default`}
                      className="rounded p-1 text-[var(--color-text-tertiary)] hover:text-[var(--color-text-primary)] transition-colors duration-150 cursor-pointer"
                    >
                      <RotateCcw className="h-3.5 w-3.5" />
                    </button>
                  )}
                </div>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
};
