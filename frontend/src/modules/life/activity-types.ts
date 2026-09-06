/** A single event from any module that happened on a given date. */
export interface ActivityEvent {
  time: string | null;
  module: string;
  action: string;
  emoji: string;
  summary: string;
  detail: string | null;
}

/** Summary counts per module. */
export interface ActivityStats {
  total_events: number;
  todos_created: number;
  todos_completed: number;
  todos_due: number;
  expenses: number;
  income: number;
  portfolio_trades: number;
  habits_done: number;
  reminders_fired: number;
  bookmarks_saved: number;
  bills_paid: number;
  footprints: number;
}

/** Response from GET /life/days/:date/activity */
export interface DayActivityResponse {
  date: string;
  events: ActivityEvent[];
  stats: ActivityStats;
}
