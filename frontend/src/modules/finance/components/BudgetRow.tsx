import React, { useState } from 'react';
import { Pencil, Trash2 } from 'lucide-react';
import { formatMinor } from '../utils';
import type { BudgetStatus } from '../types';

interface BudgetRowProps {
  status: BudgetStatus;
  onEdit: () => void;
  onRemove: () => void;
}

/** The text that carries the state, so it is never conveyed by colour alone. */
export function stateText(s: BudgetStatus): string {
  const exp = s.budget.currency_exponent;
  if (s.state === 'exceeded') {
    return s.remaining_minor < 0
      ? `Over by ${formatMinor(-s.remaining_minor, exp)}`
      : 'Limit reached';
  }
  if (s.state === 'warning') return `${s.percent_used} % used`;
  return 'On track';
}

const BAR_TONE: Record<BudgetStatus['state'], string> = {
  ok: 'bg-neutral-500 dark:bg-neutral-400',
  warning: 'bg-amber-500',
  exceeded: 'bg-rose-500',
};

const ICON_BUTTON =
  'p-1.5 rounded text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition-colors duration-150 cursor-pointer focus-visible:outline-2 focus-visible:outline-neutral-500';

export const BudgetRow: React.FC<BudgetRowProps> = ({ status, onEdit, onRemove }) => {
  const [confirming, setConfirming] = useState(false);
  const { budget, pace } = status;
  const exp = budget.currency_exponent;
  const name = budget.category_name ?? 'Overall';
  const barPct = Math.min(100, Math.max(0, status.percent_used));
  const limit = budget.limit_minor;
  const paceLeft = pace && limit > 0 ? Math.min(100, (pace.expected_minor / limit) * 100) : null;
  const paceText = pace
    ? `Expected ${formatMinor(pace.expected_minor, exp)} by today · projected ${formatMinor(pace.projected_minor, exp)}`
    : null;

  return (
    <li data-testid="budget-row" className="py-3 space-y-2">
      <div className="flex items-baseline justify-between gap-3">
        <span className="text-sm font-medium text-neutral-900 dark:text-neutral-100 font-sans truncate">
          {name} <span className="font-mono text-[11px] text-neutral-500">{budget.currency}</span>
        </span>
        <span className="text-xs font-mono tabular-nums text-neutral-700 dark:text-neutral-300 whitespace-nowrap">
          {formatMinor(status.spent_minor, exp)} / {formatMinor(limit, exp)}
        </span>
      </div>

      <div
        role="progressbar"
        aria-label={`${name} budget`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={barPct}
        aria-valuetext={paceText ? `${stateText(status)}. ${paceText}` : stateText(status)}
        className="relative h-1.5 rounded-full bg-neutral-200 dark:bg-neutral-800"
      >
        <div
          className={`h-full rounded-full transition-colors duration-150 ${BAR_TONE[status.state]}`}
          style={{ width: `${barPct}%` }}
        />
        {paceLeft !== null && (
          <div
            aria-hidden="true"
            className="absolute -top-0.5 -bottom-0.5 w-0.5 bg-neutral-900 dark:bg-neutral-100"
            style={{ left: `${paceLeft}%` }}
          />
        )}
      </div>

      <div className="flex items-center justify-between gap-3">
        <div className="text-xs font-sans text-neutral-600 dark:text-neutral-400 space-x-2">
          <span className="font-medium">{stateText(status)}</span>
          {status.state === 'ok' && <span className="font-mono">{status.percent_used} %</span>}
          {paceText && <span className="text-neutral-500">{paceText}</span>}
        </div>

        {confirming ? (
          <div className="flex items-center gap-1 text-xs font-sans">
            <span className="text-neutral-500">Remove this budget?</span>
            <button
              type="button"
              onClick={onRemove}
              className="px-2 py-1 rounded font-medium text-rose-600 dark:text-rose-400 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition-colors duration-150 cursor-pointer focus-visible:outline-2 focus-visible:outline-neutral-500"
            >
              Confirm remove
            </button>
            <button
              type="button"
              onClick={() => setConfirming(false)}
              className="px-2 py-1 rounded text-neutral-600 dark:text-neutral-400 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition-colors duration-150 cursor-pointer focus-visible:outline-2 focus-visible:outline-neutral-500"
            >
              Keep
            </button>
          </div>
        ) : (
          <div className="flex items-center">
            <button type="button" aria-label="Edit budget" title="Edit budget" onClick={onEdit} className={ICON_BUTTON}>
              <Pencil className="w-3.5 h-3.5" />
            </button>
            <button
              type="button"
              aria-label="Remove budget"
              title="Remove budget"
              onClick={() => setConfirming(true)}
              className={ICON_BUTTON}
            >
              <Trash2 className="w-3.5 h-3.5" />
            </button>
          </div>
        )}
      </div>
    </li>
  );
};
