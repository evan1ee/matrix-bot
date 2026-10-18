import type { AiToolOutcome } from './liveTurn';

export type ToolStepOutcome = 'running' | AiToolOutcome | null;

/** [finished label, running label] for the tools people see most. */
const KNOWN: Record<string, readonly [string, string]> = {
  add_todo: ['Add todo', 'Adding todo…'],
  update_todo: ['Update todo', 'Updating todo…'],
  delete_todo: ['Delete todo', 'Deleting todo…'],
  get_my_tasks: ['Look up tasks', 'Looking up tasks…'],
  list_finance_transactions: ['List transactions', 'Listing transactions…'],
  record_finance_entry: ['Record spending', 'Recording spending…'],
  cancel_reminder: ['Cancel reminder', 'Cancelling reminder…'],
  set_reminder: ['Set reminder', 'Setting reminder…'],
  log_habit: ['Log habit', 'Logging habit…'],
  get_my_portfolio_summary: ['Look up portfolio', 'Looking up portfolio…'],
  get_weekly_retrospective: ['Build retrospective', 'Building retrospective…'],
  web_search: ['Search the web', 'Searching the web…'],
};

/** `add_todo` becomes "Add todo". */
function words(name: string): string {
  const spaced = name.replace(/[_-]+/g, ' ').trim();
  return spaced ? spaced.charAt(0).toUpperCase() + spaced.slice(1) : 'Tool';
}

export function toolLabel(name: string, outcome: ToolStepOutcome): string {
  const known = KNOWN[name];
  if (outcome === 'running') return known ? known[1] : `${words(name)}…`;
  const base = known ? known[0] : words(name);
  if (outcome === 'failed') return `${base} failed`;
  if (outcome === 'needs_confirmation') return `${base} needs confirmation`;
  return base;
}
