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
            <th className="px-4 py-2.5">Current Value</th>
            <th className="px-4 py-2.5">Default</th>
            <th className="px-4 py-2.5">Hot-Reload</th>
            <th className="px-4 py-2.5">Description</th>
            <th className="px-4 py-2.5 text-right">Actions</th>
          </tr>
        </thead>
        <tbody className="divide-y divide-[var(--color-border-subtle)]">
          {settings.map((s) => (
            <tr key={s.key} className="hover:bg-[var(--color-surface-hover)] transition-colors">
              <td className="px-4 py-3 font-mono font-medium text-[var(--color-text-primary)]">
                {s.key}
              </td>
              <td className="px-4 py-3 font-mono">
                <span className={s.is_custom ? 'font-semibold text-emerald-600 dark:text-emerald-400' : 'text-[var(--color-text-secondary)]'}>
                  {typeof s.value === 'object' ? JSON.stringify(s.value) : String(s.value)}
                </span>
                {s.is_custom && (
                  <span className="ml-1.5 inline-block text-[10px] rounded bg-emerald-500/10 px-1 py-0.2 text-emerald-600 dark:text-emerald-400">
                    custom
                  </span>
                )}
              </td>
              <td className="px-4 py-3 font-mono text-[var(--color-text-tertiary)] text-[11px]">
                {typeof s.default_value === 'object' ? JSON.stringify(s.default_value) : String(s.default_value)}
              </td>
              <td className="px-4 py-3">
                <span className={`inline-flex rounded-full px-2 py-0.5 text-[10px] font-medium ${
                  s.hot_reloadable
                    ? 'bg-emerald-500/10 text-emerald-600 dark:text-emerald-400'
                    : 'bg-amber-500/10 text-amber-600 dark:text-amber-400'
                }`}>
                  {s.hot_reloadable ? 'Instant' : 'Restart'}
                </span>
              </td>
              <td className="px-4 py-3 text-[var(--color-text-secondary)] max-w-xs truncate">
                {s.description}
              </td>
              <td className="px-4 py-3 text-right">
                <div className="flex items-center justify-end gap-1.5">
                  <button
                    onClick={() => onEdit(s)}
                    className="rounded px-2 py-1 text-xs text-[var(--color-text-secondary)] hover:bg-[var(--color-surface-hover)] hover:text-[var(--color-text-primary)] cursor-pointer"
                  >
                    Edit
                  </button>
                  {s.is_custom && (
                    <button
                      onClick={() => onReset(s)}
                      title="Reset to default"
                      className="rounded p-1 text-[var(--color-text-tertiary)] hover:text-amber-500 cursor-pointer"
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
