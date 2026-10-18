import React from 'react';
import { AlertCircle, Check, ChevronRight, Loader2, ShieldQuestion } from 'lucide-react';
import { toolLabel, type ToolStepOutcome } from './toolLabels';

function OutcomeIcon({ outcome }: { outcome: ToolStepOutcome }) {
  const cls = 'w-3.5 h-3.5 shrink-0';
  switch (outcome) {
    case 'running':
      return <Loader2 aria-hidden className={`${cls} animate-spin text-neutral-500`} />;
    case 'ok':
      return <Check aria-hidden className={`${cls} text-neutral-600 dark:text-neutral-400`} />;
    case 'failed':
      return <AlertCircle aria-hidden className={`${cls} text-red-600 dark:text-red-400`} />;
    case 'needs_confirmation':
      return <ShieldQuestion aria-hidden className={`${cls} text-amber-600 dark:text-amber-400`} />;
    default:
      return null; // an old row stored before outcomes were recorded
  }
}

/**
 * One tool call as a quiet line. With a `result` (a stored row) it is a
 * collapsed disclosure that expands to the raw text; a live step has none.
 */
export const ToolStep: React.FC<{ name: string; outcome: ToolStepOutcome; result?: string }> = ({
  name,
  outcome,
  result,
}) => {
  const line = (
    <>
      <OutcomeIcon outcome={outcome} />
      <span>{toolLabel(name, outcome)}</span>
    </>
  );
  const base = 'text-xs font-sans text-neutral-600 dark:text-neutral-400';

  if (result === undefined) {
    return (
      <div data-testid="tool-step" data-outcome={outcome ?? 'none'} className={`${base} flex items-center gap-1.5 py-0.5`}>
        {line}
      </div>
    );
  }
  return (
    <details data-testid="tool-step" data-outcome={outcome ?? 'none'} className={`${base} group`}>
      <summary className="flex items-center gap-1.5 py-0.5 cursor-pointer select-none list-none [&::-webkit-details-marker]:hidden focus-visible:outline focus-visible:outline-2 focus-visible:outline-neutral-500 rounded-sm">
        <ChevronRight aria-hidden className="w-3 h-3 shrink-0 text-neutral-400 transition-transform duration-150 group-open:rotate-90" />
        {line}
      </summary>
      <div className="mt-1 max-h-40 overflow-y-auto rounded bg-neutral-200/40 dark:bg-neutral-950/50 p-2 font-mono text-[11px] leading-relaxed whitespace-pre-wrap break-all border border-neutral-200/60 dark:border-neutral-800/60 text-neutral-800 dark:text-neutral-200">
        {result}
      </div>
    </details>
  );
};
