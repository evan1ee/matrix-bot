import { createHash } from 'node:crypto';
import type { Page, Route } from '@playwright/test';
import {
  auditPage,
  configStatus,
  initialAdminSettings,
  type FakeAuditRow,
  type FakeSecret,
  type FakeSetting,
} from './admin-fixtures';
import { installAiStreamStub, type StreamScript } from './ai-stream-stub';

/**
 * An in-memory stand-in for `/api/v1`, installed as a Playwright route so the
 * browser never reaches the network.
 *
 * It mirrors the real contract closely enough that the UI cannot tell the
 * difference — including the validation rules the SPA is supposed to respect
 * (`api/todos.rs`), so a test can prove the UI surfaces a server rejection
 * rather than silently swallowing it.
 */

export interface FakeTodo {
  id: number;
  room_id: string;
  sender_mxid: string;
  sender_name: string;
  text: string;
  priority: string;
  due_at: string | null;
  remind_before_minutes: number | null;
  completed: boolean;
  created_at: string;
}

export interface FakeReminder {
  id: number;
  room_id: string;
  sender_mxid: string;
  sender_name: string;
  message: string;
  trigger_at: string;
  recurrence: string;
  status: string;
  retry_count: number;
  todo_id: number | null;
  created_at: string;
}

export interface FakeTransaction {
  id: number;
  room_id: string;
  sender_mxid: string;
  sender_name: string;
  direction: string;
  amount_minor: number;
  currency: string;
  currency_exponent: number;
  category_id: number | null;
  category_name: string | null;
  note: string | null;
  occurred_at: string;
  created_at: string;
  source_kind?: string | null;
  source_id?: number | null;
}

export interface FakeRecurringBill {
  id: number;
  room_id: string;
  owner_id: string;
  sender_name: string;
  name: string;
  category: string;
  amount_minor: number;
  currency: string;
  currency_exponent: number;
  frequency: string;
  due_day: number | null;
  next_due_at: string;
  total_installments: number | null;
  paid_installments: number;
  auto_pay: boolean;
  status: string;
  notes: string | null;
  remind_before_minutes: number | null;
  remind_room_id: string | null;
  created_at: string;
  updated_at: string;
}

export interface FakeCategory {
  id: number;
  owner_id: string;
  name: string;
  created_at: string;
}

export interface FakeRoom {
  room_id: string;
  name: string;
}

export interface FakeCashBalance {
  id: number;
  owner_id: string;
  currency: string;
  currency_exponent: number;
  balance_minor: number;
  updated_at: string;
}

export interface FakeHolding {
  id: number;
  owner_id: string;
  symbol: string;
  quantity_minor: number;
  avg_cost_minor: number;
  currency: string;
  currency_exponent: number;
  current_price_minor: number | null;
  price_updated_at: string | null;
  note: string | null;
  created_at: string;
  updated_at: string;
}

export interface FakeFootprint {
  id: number;
  owner_id: string;
  title: string;
  latitude: number;
  longitude: number;
  country: string | null;
  city: string | null;
  address: string | null;
  arrival_date: string;
  departure_date: string | null;
  stay_duration_days: number;
  spend_amount_minor: number;
  spend_currency: string;
  spend_currency_exponent: number;
  rating: number;
  category: string;
  note: string | null;
  created_at: string;
  updated_at: string;
}

export interface FakeHabit {
  id: number;
  owner_id: string;
  name: string;
  description: string | null;
  frequency: string;
  category: string;
  target_days_per_week: number;
  archived: boolean;
  created_at: string;
  updated_at: string;
}

export interface FakeHabitLog {
  id: number;
  habit_id: number;
  owner_id: string;
  log_date: string;
  completed: boolean;
  note: string | null;
  created_at: string;
}

export interface FakeLifeProfile {
  id: number;
  owner_id: string;
  birth_date: string;
  expected_lifespan_years: number;
  created_at: string;
  updated_at: string;
}

export interface FakeLifeDayLog {
  id: number;
  owner_id: string;
  log_date: string;
  mood: string;
  weather: string | null;
  activities: string | null;
  note: string | null;
  time_allocation?: string | null;
  created_at: string;
  updated_at: string;
}

export interface FakeWebhook {
  id: string;
  owner_id: string;
  name: string;
  token_prefix: string;
  default_room_id: string;
  is_active: boolean;
  total_delivered_count: number;
  failure_count: number;
  last_triggered_at: string | null;
  created_at: string;
}

export interface FakeWebhookOutboxItem {
  id: string;
  webhook_id: string;
  owner_id: string;
  room_id: string;
  severity: string;
  title: string | null;
  html_body: string;
  plain_body: string;
  status: string;
  attempts: number;
  max_attempts: number;
  next_attempt_at: string;
  last_error: string | null;
  created_at: string;
  delivered_at: string | null;
}

export interface FakeBookmark {
  id: number;
  owner_id: string;
  url: string;
  title: string;
  description: string | null;
  author: string | null;
  image_url: string | null;
  domain: string;
  reader_content: string | null;
  reading_time_minutes: number;
  tags: string;
  is_favorite: boolean;
  is_archived: boolean;
  created_at: string;
  updated_at: string;
}

export interface FakeNote {
  id: number;
  owner_id: string;
  title: string;
  content: string;
  note_type: string;
  source_channel: string;
  source_ref: string | null;
  tags: string[];
  is_archived: boolean;
  created_at: string;
  updated_at: string;
}

/** The note's body hash, as the server computes it (SHA-256 of the Markdown). */
function noteHash(content: string): string {
  return createHash('sha256').update(content, 'utf8').digest('hex');
}

/** What `GET /notes` returns for one note: metadata and an excerpt, no body. */
function noteSummary(n: FakeNote) {
  const { content, ...rest } = n;
  return {
    ...rest,
    excerpt: content.replace(/\s+/g, ' ').trim().slice(0, 200),
    content_hash: noteHash(content),
    indexed_at: null,
  };
}

/** What `GET /notes/:id` returns: the summary plus the body. */
function noteDocument(n: FakeNote) {
  return { ...noteSummary(n), content: n.content, format: 'markdown' };
}

export interface FakeAgentRun {
  id: string;
  scheduled_task_id: string | null;
  account_id: string;
  task_name: string;
  goal: string;
  origin_conversation: string | null;
  status: string;
  max_steps: number;
  current_step: number;
  total_tokens: number;
  error: string | null;
  started_at: string;
  completed_at: string | null;
  updated_at: string;
}

export interface FakeAgentStep {
  id: string;
  run_id: string;
  step_number: number;
  thought: string | null;
  tool_name: string | null;
  tool_input: any;
  tool_output: any;
  status: string;
  tokens_used: number;
  created_at: string;
}

export interface FakeApprovalRequest {
  id: string;
  run_id: string;
  account_id: string;
  tool_name: string;
  tool_input: any;
  status: string;
  expires_at: string;
  resolved_at: string | null;
  created_at: string;
}

export interface FakeScheduledTask {
  id: string;
  account_id: string;
  task_id: string;
  cron_expression: string;
  enabled: boolean;
  config: any;
  last_run_at: string | null;
  next_run_at: string;
  created_at: string;
  updated_at: string;
}

export interface FakeAiConfig {
  owner_id: string;
  api_base_url: string | null;
  api_key: string | null;
  model_name: string | null;
  system_prompt: string | null;
  temperature: number | null;
  max_tokens: number | null;
  updated_at: string;
  has_api_key?: boolean;
}

export interface FakeAiConversation {
  id: string;
  owner_id: string;
  room_id: string;
  created_at: string;
  updated_at: string;
}

export interface FakeAiMessage {
  id: string;
  conversation_id: string;
  role: 'system' | 'user' | 'assistant' | 'tool';
  content: string;
  tool_call_id: string | null;
  tool_name: string | null;
  created_at: string;
}

const OWNER = 'acct_00000000000000000000000000000001';
const DISPLAY_NAME = 'tester';
/** Mirrors `due_reminder::MAX_LEAD_MINUTES`. */
const MAX_LEAD_MINUTES = 60 * 24 * 30;
/** Mirrors `portfolio::models::QUANTITY_EXPONENT`. */
const QUANTITY_EXPONENT = 8;

export class FakeApi {
  todos: FakeTodo[] = [];
  reminders: FakeReminder[] = [];
  webhooks: FakeWebhook[] = [];
  webhookLogs: FakeWebhookOutboxItem[] = [];
  aiConfig: FakeAiConfig = {
    owner_id: OWNER,
    api_base_url: 'https://api.openai.com/v1',
    api_key: null,
    model_name: 'gpt-4o-mini',
    system_prompt: null,
    temperature: 0.7,
    max_tokens: 2048,
    updated_at: new Date().toISOString(),
  };
  aiConversations: FakeAiConversation[] = [];
  aiMessages: FakeAiMessage[] = [];
  nextWebhookId = 1;
  nextLogId = 1;
  nextAiConvId = 1;
  nextAiMsgId = 1;
  notes: FakeNote[] = [];
  /** The `X-Search-Mode` header `POST /knowledge/search` answers with. */
  searchMode: 'hybrid' | 'lexical' = 'hybrid';
  /** What `POST /notes/import` answers with. */
  importReport: {
    created: number;
    updated: number;
    skipped: { file: string; reason: string }[];
  } = { created: 0, updated: 0, skipped: [] };
  /** File names of the zips `POST /notes/import` received. */
  importedFileNames: string[] = [];
  nextNoteId = 1;
  rooms: FakeRoom[] = [
    { room_id: '!general:mbm.one', name: 'General' },
    { room_id: '!ops:mbm.one', name: 'Ops' },
  ];

  /** Requests seen, so tests can assert on what the UI actually sent. */
  requests: { method: string; url: string; body: unknown }[] = [];

  /** Set to make the next matching call fail, for error-path tests. */
  failNext: { path: RegExp; status: number; error: string } | null = null;

  /**
   * Whether `/auth/me` answers.
   *
   * This is the whole of "am I signed in?" now. The real session is an HttpOnly
   * cookie the page cannot read, so the app asks the server and believes the
   * answer — there is no client-side token for a test to plant.
   */
  signedIn = true;
  role: 'user' | 'admin' = 'user';
  adminSettings: FakeSetting[] = initialAdminSettings();
  adminSecrets: FakeSecret[] = [];
  adminAuditLogs: FakeAuditRow[] = [];
  nextSecretId = 1;

  /** Passkeys the account holds, as Settings -> Security lists them. */
  passkeys: { id: string; label: string; created_at: string; last_used_at: string | null }[] = [
    { id: 'cred_1', label: 'MacBook Touch ID', created_at: '2026-01-01T10:00:00Z', last_used_at: '2026-08-01T09:00:00Z' },
  ];

  /** Matrix identities linked to the account. */
  linkedMxids: string[] = [];

  /** Signed-in devices. */
  sessions: { id: string; created_at: string; last_seen_at: string; user_agent: string | null; current: boolean }[] = [
    { id: 'sess_current', created_at: '2026-08-01T10:00:00Z', last_seen_at: '2026-08-12T09:00:00Z', user_agent: 'Chrome on macOS', current: true },
  ];

  /** Entries for the account's own audit log. */
  securityEvents: { id: number; kind: string; detail: string | null; created_at: string }[] = [
    { id: 1, kind: 'signed_in', detail: null, created_at: '2026-08-12T09:00:00Z' },
  ];

  recoveryStatus: { codes_remaining: number; recovery_valid_after: string | null } = {
    codes_remaining: 10,
    recovery_valid_after: null,
  };

  /**
   * Events waiting to be delivered on the next `/events` connection.
   *
   * The stream is the only way the app learns about a change it did not make
   * itself — a Matrix command, or another device — so this is what lets a test
   * drive that path.
   */
  pendingEvents: unknown[] = [];

  /** Linked identities on channels other than Matrix (Matrix ones are `linkedMxids`). */
  otherIdentities: { channel: string; user_id: string; display_name: string | null; linked_at: string }[] = [];

  /**
   * What `POST /link/inspect` answers for a token the table below does not know.
   * The link page reads the token from the address bar, so a spec picks its
   * state by choosing the token.
   */
  linkPreview: {
    state: string;
    channel: string | null;
    user_id: string | null;
    merge_item_count: number | null;
  } = { state: 'bind', channel: 'telegram', user_id: '42', merge_item_count: null };

  /** Token-driven answers for `POST /link/inspect`. */
  linkPreviews: Record<string, { state: string; channel: string | null; user_id: string | null; merge_item_count: number | null }> = {
    'expired-token': { state: 'expired', channel: null, user_id: null, merge_item_count: null },
    'signup-token': { state: 'signup_allowed', channel: 'telegram', user_id: '77', merge_item_count: null },
    'merge-token': { state: 'needs_merge', channel: 'discord', user_id: '9001', merge_item_count: 7 },
    'signin-token': { state: 'sign_in_required', channel: 'telegram', user_id: '42', merge_item_count: null },
    'yours-token': { state: 'already_yours', channel: 'telegram', user_id: '42', merge_item_count: null },
    'taken-token': { state: 'owned_elsewhere', channel: 'telegram', user_id: '42', merge_item_count: null },
  };

  /** What `POST /link/claim` answers; `merge-token` claims merge. */
  claimOutcome = 'linked';

  transactions: FakeTransaction[] = [];
  categories: FakeCategory[] = [];
  financeShare: {
    owner_id: string;
    token: string;
    is_enabled: boolean;
    title: string | null;
    created_at: string;
    updated_at: string;
  } | null = null;
  holdings: FakeHolding[] = [];
  cash: FakeCashBalance[] = [];
  alerts: any[] = [];
  footprints: FakeFootprint[] = [];
  habits: FakeHabit[] = [];
  habitLogs: FakeHabitLog[] = [];
  lifeProfile: FakeLifeProfile | null = null;
  lifeDayLogs: FakeLifeDayLog[] = [];
  bills: FakeRecurringBill[] = [];
  bookmarks: FakeBookmark[] = [];
  agentRuns: FakeAgentRun[] = [
    {
      id: 'run-1',
      scheduled_task_id: null,
      account_id: 'acct_00000000000000000000000000000001',
      task_name: 'ad_hoc_goal',
      goal: 'Audit monthly expenses and categorize unassigned items',
      origin_conversation: 'web',
      status: 'completed',
      max_steps: 10,
      current_step: 3,
      total_tokens: 1450,
      error: null,
      started_at: '2026-10-04T00:00:00Z',
      completed_at: '2026-10-04T00:00:30Z',
      updated_at: '2026-10-04T00:00:30Z',
    },
    {
      id: 'run-2',
      scheduled_task_id: null,
      account_id: 'acct_00000000000000000000000000000001',
      task_name: 'ad_hoc_goal',
      goal: 'Deposit funds and buy AAPL shares',
      origin_conversation: 'web',
      status: 'waiting_approval',
      max_steps: 10,
      current_step: 2,
      total_tokens: 820,
      error: null,
      started_at: '2026-10-04T00:10:00Z',
      completed_at: null,
      updated_at: '2026-10-04T00:10:15Z',
    },
  ];

  agentSteps: Record<string, FakeAgentStep[]> = {
    'run-1': [
      {
        id: 'step-1-1',
        run_id: 'run-1',
        step_number: 1,
        thought: 'I need to fetch recent transactions from the finance service.',
        tool_name: 'query_transactions',
        tool_input: { limit: 20 },
        tool_output: [{ id: 101, note: 'Coffee', amount: 4.5 }],
        status: 'completed',
        tokens_used: 450,
        created_at: '2026-10-04T00:00:05Z',
      },
      {
        id: 'step-1-2',
        run_id: 'run-1',
        step_number: 2,
        thought: 'Categorizing uncategorized coffee expense as Dining.',
        tool_name: 'categorize_transaction',
        tool_input: { id: 101, category: 'Dining' },
        tool_output: { success: true },
        status: 'completed',
        tokens_used: 520,
        created_at: '2026-10-04T00:00:15Z',
      },
      {
        id: 'step-1-3',
        run_id: 'run-1',
        step_number: 3,
        thought: 'All expenses are audited. Synthesizing summary.',
        tool_name: null,
        tool_input: null,
        tool_output: 'Audited 1 expense and categorized as Dining.',
        status: 'completed',
        tokens_used: 480,
        created_at: '2026-10-04T00:00:25Z',
      },
    ],
    'run-2': [
      {
        id: 'step-2-1',
        run_id: 'run-2',
        step_number: 1,
        thought: 'Checking current balance before depositing.',
        tool_name: 'get_portfolio_cash',
        tool_input: { currency: 'USD' },
        tool_output: { balance: 500 },
        status: 'completed',
        tokens_used: 320,
        created_at: '2026-10-04T00:10:05Z',
      },
      {
        id: 'step-2-2',
        run_id: 'run-2',
        step_number: 2,
        thought: 'Executing portfolio deposit of 1000 USD is a sensitive action requiring user authorization.',
        tool_name: 'execute_portfolio_deposit',
        tool_input: { amount_minor: 100000, currency: 'USD' },
        tool_output: null,
        status: 'executing',
        tokens_used: 500,
        created_at: '2026-10-04T00:10:15Z',
      },
    ],
  };

  agentApprovals: FakeApprovalRequest[] = [
    {
      id: 'appr-1',
      run_id: 'run-2',
      account_id: 'acct_00000000000000000000000000000001',
      tool_name: 'execute_portfolio_deposit',
      tool_input: { amount_minor: 100000, currency: 'USD' },
      status: 'pending',
      expires_at: '2026-10-04T00:25:15Z',
      resolved_at: null,
      created_at: '2026-10-04T00:10:15Z',
    },
  ];

  agentTasks: FakeScheduledTask[] = [
    {
      id: 'st-1',
      account_id: 'acct_00000000000000000000000000000001',
      task_id: 'WeeklyBookmarkDigestTask',
      cron_expression: '0 9 * * 1',
      enabled: true,
      config: { max_items: 20 },
      last_run_at: '2026-09-28T09:00:00Z',
      next_run_at: '2026-10-05T09:00:00Z',
      created_at: '2026-09-20T00:00:00Z',
      updated_at: '2026-09-20T00:00:00Z',
    },
    {
      id: 'st-2',
      account_id: 'acct_00000000000000000000000000000001',
      task_id: 'MonthlyFinancialAuditTask',
      cron_expression: '0 8 1 * *',
      enabled: true,
      config: { include_cashflow: true },
      last_run_at: '2026-10-01T08:00:00Z',
      next_run_at: '2026-11-01T08:00:00Z',
      created_at: '2026-09-20T00:00:00Z',
      updated_at: '2026-09-20T00:00:00Z',
    },
  ];
  /** Mirrors the server default (`modules::settings::UserSettings::default`). */
  settings: { timezone: string; currency: string } = { timezone: 'UTC', currency: 'USD' };

  private nextTodoId = 1;
  private nextReminderId = 1;
  private nextTxId = 1;
  private nextCategoryId = 1;
  private nextHoldingId = 1;
  private nextCashId = 1;
  private nextFootprintId = 1;
  private nextHabitId = 1;
  private nextHabitLogId = 1;
  private nextLifeProfileId = 1;
  private nextLifeDayLogId = 1;
  private nextBillId = 1;
  private nextBookmarkId = 1;

  seedBookmark(partial: Partial<FakeBookmark> & { url: string; title?: string }): FakeBookmark {
    const id = this.nextBookmarkId++;
    const now = new Date().toISOString();
    let domain = 'example.com';
    try {
      domain = new URL(partial.url).hostname;
    } catch (_) {}
    const bookmark: FakeBookmark = {
      id,
      owner_id: OWNER,
      url: partial.url,
      title: partial.title || 'Saved Page',
      description: partial.description ?? null,
      author: partial.author ?? null,
      image_url: partial.image_url ?? null,
      domain: partial.domain ?? domain,
      reader_content: partial.reader_content ?? null,
      reading_time_minutes: partial.reading_time_minutes ?? 3,
      tags: partial.tags ?? '',
      is_favorite: partial.is_favorite ?? false,
      is_archived: partial.is_archived ?? false,
      created_at: partial.created_at ?? now,
      updated_at: partial.updated_at ?? now,
    };
    this.bookmarks.push(bookmark);
    return bookmark;
  }

  seedNote(partial: Partial<FakeNote> = {}): FakeNote {
    const id = partial.id ?? this.nextNoteId++;
    const now = new Date().toISOString();
    const note: FakeNote = {
      id,
      owner_id: partial.owner_id ?? OWNER,
      title: partial.title ?? 'Test Note',
      content: partial.content ?? 'Note content',
      note_type: partial.note_type ?? 'note',
      source_channel: partial.source_channel ?? 'web',
      source_ref: partial.source_ref ?? null,
      tags: partial.tags ?? [],
      is_archived: partial.is_archived ?? false,
      created_at: partial.created_at ?? now,
      updated_at: partial.updated_at ?? now,
    };
    this.notes.push(note);
    return note;
  }

  seedLifeProfile(partial: Partial<FakeLifeProfile> = {}): FakeLifeProfile {
    const profile: FakeLifeProfile = {
      id: this.nextLifeProfileId++,
      owner_id: OWNER,
      birth_date: partial.birth_date || '1995-05-15',
      expected_lifespan_years: partial.expected_lifespan_years || 80,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    };
    this.lifeProfile = profile;
    return profile;
  }

  seedLifeDayLog(partial: Partial<FakeLifeDayLog> = {}): FakeLifeDayLog {
    const log: FakeLifeDayLog = {
      id: this.nextLifeDayLogId++,
      owner_id: OWNER,
      log_date: partial.log_date || '2026-06-01',
      mood: partial.mood || 'good',
      weather: partial.weather ?? 'sunny',
      activities: partial.activities ?? JSON.stringify(['Workout', 'Deep Work']),
      note: partial.note ?? 'Great day of progress!',
      time_allocation: partial.time_allocation ?? null,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    };
    this.lifeDayLogs.push(log);
    return log;
  }

  seedHabit(partial: Partial<FakeHabit> = {}): FakeHabit {
    const habit: FakeHabit = {
      id: this.nextHabitId++,
      owner_id: OWNER,
      name: partial.name || `Habit #${this.nextHabitId}`,
      description: partial.description ?? null,
      frequency: partial.frequency || 'daily',
      category: partial.category || 'wellness',
      target_days_per_week: partial.target_days_per_week || 7,
      archived: partial.archived || false,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    };
    this.habits.push(habit);
    return habit;
  }

  seedHabitLog(partial: Partial<FakeHabitLog> = {}): FakeHabitLog {
    const log: FakeHabitLog = {
      id: this.nextHabitLogId++,
      habit_id: partial.habit_id || 1,
      owner_id: OWNER,
      log_date: partial.log_date || new Date().toISOString().split('T')[0],
      completed: partial.completed ?? true,
      note: partial.note ?? null,
      created_at: new Date().toISOString(),
    };
    this.habitLogs.push(log);
    return log;
  }

  seedFootprint(partial: Partial<FakeFootprint> = {}): FakeFootprint {
    const arr = partial.arrival_date ?? '2026-06-01';
    const dep = partial.departure_date ?? null;
    let duration = 1;
    if (dep) {
      const diff = Math.round((new Date(dep).getTime() - new Date(arr).getTime()) / (1000 * 3600 * 24));
      duration = Math.max(1, diff);
    }
    const fp: FakeFootprint = {
      id: this.nextFootprintId++,
      owner_id: OWNER,
      title: partial.title ?? 'Eiffel Tower',
      latitude: partial.latitude ?? 48.8584,
      longitude: partial.longitude ?? 2.2945,
      country: partial.country ?? 'France',
      city: partial.city ?? 'Paris',
      address: partial.address ?? 'Champ de Mars, 5 Av. Anatole France, 75007 Paris, France',
      arrival_date: arr,
      departure_date: dep,
      stay_duration_days: partial.stay_duration_days ?? duration,
      spend_amount_minor: partial.spend_amount_minor ?? 0,
      spend_currency: partial.spend_currency ?? 'USD',
      spend_currency_exponent:
        partial.spend_currency_exponent ?? exponentFor(partial.spend_currency ?? 'USD'),
      rating: partial.rating ?? 5,
      category: partial.category ?? 'sightseeing',
      note: partial.note ?? null,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    };
    this.footprints.push(fp);
    return fp;
  }

  seedTodo(partial: Partial<FakeTodo> = {}): FakeTodo {
    const todo: FakeTodo = {
      id: this.nextTodoId++,
      room_id: 'web',
      sender_mxid: OWNER,
      sender_name: DISPLAY_NAME,
      text: 'Seeded task',
      priority: 'medium',
      due_at: null,
      remind_before_minutes: null,
      completed: false,
      created_at: new Date().toISOString(),
      ...partial,
    };
    this.todos.push(todo);
    return todo;
  }

  seedReminder(partial: Partial<FakeReminder> = {}): FakeReminder {
    const reminder: FakeReminder = {
      id: this.nextReminderId++,
      room_id: '!general:mbm.one',
      sender_mxid: OWNER,
      sender_name: DISPLAY_NAME,
      message: 'Seeded reminder',
      trigger_at: new Date(Date.now() + 3_600_000).toISOString(),
      recurrence: 'none',
      status: 'pending',
      retry_count: 0,
      todo_id: null,
      created_at: new Date().toISOString(),
      ...partial,
    };
    this.reminders.push(reminder);
    return reminder;
  }

  seedWebhook(partial: Partial<FakeWebhook> = {}): FakeWebhook {
    const id = `whk_${this.nextWebhookId++}`;
    const webhook: FakeWebhook = {
      id,
      owner_id: OWNER,
      name: 'Seeded Webhook',
      token_prefix: 'whk_a1b2c3',
      default_room_id: '!general:mbm.one',
      is_active: true,
      total_delivered_count: 0,
      failure_count: 0,
      last_triggered_at: null,
      created_at: new Date().toISOString(),
      ...partial,
    };
    this.webhooks.push(webhook);
    return webhook;
  }

  seedWebhookLog(webhookId: string, partial: Partial<FakeWebhookOutboxItem> = {}): FakeWebhookOutboxItem {
    const id = `outbox_${this.nextLogId++}`;
    const log: FakeWebhookOutboxItem = {
      id,
      webhook_id: webhookId,
      owner_id: OWNER,
      room_id: '!general:mbm.one',
      severity: 'info',
      title: 'Alert Title',
      html_body: '<b>Alert Title</b><br/>Alert details here',
      plain_body: 'Alert details here',
      status: 'delivered',
      attempts: 1,
      max_attempts: 10,
      next_attempt_at: new Date().toISOString(),
      last_error: null,
      created_at: new Date().toISOString(),
      delivered_at: new Date().toISOString(),
      ...partial,
    };
    this.webhookLogs.push(log);
    return log;
  }

  seedCategory(name: string): FakeCategory {
    const category: FakeCategory = {
      id: this.nextCategoryId++,
      owner_id: OWNER,
      name,
      created_at: new Date().toISOString(),
    };
    this.categories.push(category);
    return category;
  }

  seedTransaction(partial: Partial<FakeTransaction> = {}): FakeTransaction {
    const tx: FakeTransaction = {
      id: this.nextTxId++,
      room_id: 'web',
      sender_mxid: OWNER,
      sender_name: DISPLAY_NAME,
      direction: 'expense',
      amount_minor: 1000,
      currency: 'EUR',
      currency_exponent: 2,
      category_id: null,
      category_name: null,
      note: 'Seeded entry',
      occurred_at: new Date().toISOString(),
      created_at: new Date().toISOString(),
      ...partial,
    };
    this.transactions.push(tx);
    return tx;
  }

  seedRecurringBill(partial: Partial<FakeRecurringBill> = {}): FakeRecurringBill {
    const bill: FakeRecurringBill = {
      id: this.nextBillId++,
      room_id: 'web',
      owner_id: OWNER,
      sender_name: DISPLAY_NAME,
      name: 'Netflix',
      category: 'subscription',
      amount_minor: 1599,
      currency: 'USD',
      currency_exponent: exponentFor(partial.currency ?? 'USD'),
      frequency: 'monthly',
      due_day: 15,
      next_due_at: new Date(Date.now() + 86400000 * 7).toISOString(),
      total_installments: null,
      paid_installments: 0,
      auto_pay: false,
      status: 'active',
      notes: null,
      remind_before_minutes: null,
      remind_room_id: null,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
      ...partial,
    };
    this.bills.push(bill);
    return bill;
  }

  seedHolding(partial: Partial<FakeHolding> = {}): FakeHolding {
    const holding: FakeHolding = {
      id: this.nextHoldingId++,
      owner_id: OWNER,
      symbol: 'AAPL',
      quantity_minor: 10 * 10 ** QUANTITY_EXPONENT,
      avg_cost_minor: 15000,
      currency: 'USD',
      currency_exponent: 2,
      current_price_minor: null,
      price_updated_at: null,
      note: null,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
      ...partial,
    };
    this.holdings.push(holding);
    return holding;
  }

  /** Cash on hand, so a test can fund a buy without going through the deposit
   *  UI/route first. Adds to an existing balance for the currency if present,
   *  mirroring the real `deposit_cash`'s upsert-and-add. */
  seedCash(currency: string, amountMinor: number, exponent = 2): FakeCashBalance {
    const existing = this.cash.find((c) => c.currency === currency);
    if (existing) {
      existing.balance_minor += amountMinor;
      existing.updated_at = new Date().toISOString();
      return existing;
    }
    const balance: FakeCashBalance = {
      id: this.nextCashId++,
      owner_id: OWNER,
      currency,
      currency_exponent: exponent,
      balance_minor: amountMinor,
      updated_at: new Date().toISOString(),
    };
    this.cash.push(balance);
    return balance;
  }

  /** The pending reminder linked to a todo, mirroring `find_pending_for_todo`. */
  reminderForTodo(todoId: number): FakeReminder | undefined {
    return this.reminders.find((r) => r.todo_id === todoId && r.status === 'pending');
  }

  /**
   * Reconciles a todo's linked reminder, mirroring `due_reminder::sync` — the
   * behaviour the UI depends on when it omits `remind_room_id` on an edit.
   */
  private syncDueReminder(todo: FakeTodo, explicitRoom?: string | null) {
    const existing = this.reminderForTodo(todo.id);
    const wants = Boolean(todo.due_at && todo.remind_before_minutes && !todo.completed);

    if (!wants) {
      if (existing) existing.status = 'cancelled';
      return;
    }

    const triggerAt = new Date(
      new Date(todo.due_at!).getTime() - todo.remind_before_minutes! * 60_000
    );

    if (triggerAt.getTime() <= Date.now()) {
      if (existing) existing.status = 'cancelled';
      return;
    }

    const room = explicitRoom || existing?.room_id || (todo.room_id !== 'web' ? todo.room_id : null);
    if (!room) return; // caller already rejected in `assertRoom`

    if (existing) existing.status = 'cancelled';
    this.seedReminder({
      room_id: room,
      message: `Task #${todo.id} is due: ${todo.text}`,
      trigger_at: triggerAt.toISOString(),
      todo_id: todo.id,
    });
  }

  /**
   * Queues a `SystemEvent` for the next stream connection.
   *
   * The shape matters: the server sends *unnamed* messages whose body carries
   * `type` and `payload`, and the frontend discriminates on `type`. Getting
   * that wrong is silent — the browser fires `onmessage`, the app finds no
   * matching handler, and nothing happens.
   */
  pushEvent(type: string, payload: unknown) {
    this.pendingEvents.push({ type, payload });
  }

  /** The page `install` was called with; streamed AI turns are driven through it. */
  private streamPage: Page | null = null;

  /**
   * Queues a script for the next `POST /ai/chat/stream` and returns handles to
   * keep driving it. A script with `hold` stays open until `send`/`end`.
   */
  async scriptStream(script: StreamScript) {
    const page = this.streamPage;
    if (!page) throw new Error('scriptStream needs install(page) first');
    await page.evaluate((s) => window.__aiStream.scripts.push(s), script);
    return {
      /** Sends one more `AiStreamEvent` on the held stream. */
      send: (event: Record<string, unknown>) => page.evaluate((e) => window.__aiStream.send(e), event),
      /** Ends the held stream without a terminal event. */
      end: () => page.evaluate(() => window.__aiStream.end()),
    };
  }

  /** How many scripted streams the page aborted (Stop, or a cleared turn). */
  streamAborts(): Promise<number> {
    if (!this.streamPage) throw new Error('streamAborts needs install(page) first');
    return this.streamPage.evaluate(() => window.__aiStream.aborts);
  }

  /** The request bodies the scripted streams received, in order. */
  streamRequests(): Promise<unknown[]> {
    if (!this.streamPage) throw new Error('streamRequests needs install(page) first');
    return this.streamPage.evaluate(() => window.__aiStream.bodies);
  }

  async install(page: Page) {
    this.streamPage = page;
    await page.addInitScript(installAiStreamStub);
    // Liveness/readiness are infrastructure surface, not application API: they
    // are mounted outside `/api/v1`, so they need their own route rather than
    // falling under the `/api/v1/**` interception below.
    await page.route('**/health/live', async (route) => {
      return json(route, 200, { status: 'live' });
    });
    await page.route('**/health/ready', async (route) => {
      return json(route, 200, {
        status: 'ready',
        checks: {
          database: { ok: true },
          matrix_sync: { ok: true, last_sync_at: new Date().toISOString() },
        },
      });
    });

    await page.route('**/api/v1/**', async (route) => {
      const request = route.request();
      const url = new URL(request.url());
      const path = url.pathname.replace('/api/v1', '');
      const method = request.method();

      let body: unknown = null;
      try {
        body = request.postDataJSON();
      } catch {
        body = null;
      }
      this.requests.push({ method, url: path, body });

      if (this.failNext && this.failNext.path.test(path)) {
        const { status, error } = this.failNext;
        this.failNext = null;
        return json(route, status, { error, code: status });
      }

      return this.handle(route, method, path, body as Record<string, any> | null, url);
    });
  }

  private async handle(
    route: Route,
    method: string,
    path: string,
    body: Record<string, any> | null,
    url?: URL
  ) {
    // --- auth ---
    // The passkey ceremonies are answered with canned challenges. A real
    // authenticator (Playwright's CDP virtual one) still runs, so what these
    // specs exercise is the browser half: the base64url conversions, the
    // options mapping, and how the UI reacts. Verifying an assertion is the
    // server's job and is covered by the Rust tests.
    if (path === '/auth/passkey/enroll/start' && method === 'POST') {
      if (!body?.token || body.token === 'invalid') {
        return json(route, 400, { error: 'Enrollment link is invalid or has expired.', code: 400 });
      }
      return json(route, 200, {
        ceremony_id: 'ceremony_enroll',
        options: { publicKey: registrationOptions(DISPLAY_NAME) },
      });
    }
    if (path === '/account/passkeys/add/start' && method === 'POST') {
      return json(route, 200, {
        ceremony_id: 'ceremony_add_passkey',
        options: { publicKey: registrationOptions(DISPLAY_NAME) },
      });
    }
    if (path === '/auth/passkey/register/start' && method === 'POST') {
      return json(route, 200, {
        ceremony_id: 'ceremony_register',
        options: { publicKey: registrationOptions(String(body?.display_name ?? DISPLAY_NAME)) },
      });
    }
    if (path === '/auth/passkey/register/finish' && method === 'POST') {
      this.signedIn = true;
      this.passkeys.push({
        id: `pk_${Date.now()}`,
        label: body?.label || 'Passkey',
        created_at: new Date().toISOString(),
        last_used_at: new Date().toISOString(),
      });
      return json(route, 200, { account_id: OWNER, display_name: DISPLAY_NAME });
    }
    if (path === '/auth/passkey/login/start' && method === 'POST') {
      return json(route, 200, {
        ceremony_id: 'ceremony_login',
        options: { publicKey: authenticationOptions() },
      });
    }
    if (path === '/auth/passkey/login/finish' && method === 'POST') {
      this.signedIn = true;
      return json(route, 200, { account_id: OWNER, display_name: DISPLAY_NAME });
    }
    if (path === '/auth/logout' && method === 'POST') {
      this.signedIn = false;
      return route.fulfill({ status: 204, body: '' });
    }
    if (path === '/auth/me') {
      if (!this.signedIn) {
        return json(route, 401, { error: 'Not signed in', code: 401 });
      }
      return json(route, 200, {
        account_id: OWNER,
        username: DISPLAY_NAME,
        display_name: DISPLAY_NAME,
        avatar_url: null,
        linked_mxids: this.linkedMxids,
        role: this.role,
      });
    }

    // --- admin endpoints ---
    if (path.startsWith('/admin/')) {
      if (!this.signedIn) {
        return json(route, 401, { error: 'Not signed in', code: 401 });
      }
      if (this.role !== 'admin') {
        return json(route, 403, { error: 'Admin privileges required', code: 403 });
      }

      if (path === '/admin/config-status' && method === 'GET') {
        return json(route, 200, configStatus(this.adminSettings, this.adminSecrets));
      }

      if (path === '/admin/settings' && method === 'GET') {
        return json(route, 200, this.adminSettings);
      }

      if (path.startsWith('/admin/settings/') && method === 'PUT') {
        const key = decodeURIComponent(path.slice('/admin/settings/'.length));
        const val = body?.value;
        let setting = this.adminSettings.find((s) => s.key === key);
        if (!setting) {
          setting = {
            key,
            value: val,
            default_value: val,
            value_type: typeof val === 'boolean' ? 'boolean' : typeof val === 'number' ? 'integer' : 'string',
            description: key,
            is_custom: true,
            hot_reloadable: true,
            category: 'General',
            updated_at: new Date().toISOString(),
          };
          this.adminSettings.push(setting);
        } else {
          setting.value = val;
          setting.is_custom = true;
          setting.updated_at = new Date().toISOString();
        }
        this.adminAuditLogs.unshift({
          id: this.adminAuditLogs.length + 1,
          actor_id: OWNER,
          action: 'setting_updated',
          target_type: 'setting',
          target_key: key,
          outcome: 'success',
          detail: { new_value: val },
          created_at: new Date().toISOString(),
        });
        return json(route, 200, setting);
      }

      if (path.startsWith('/admin/settings/') && method === 'DELETE') {
        const key = decodeURIComponent(path.slice('/admin/settings/'.length));
        const setting = this.adminSettings.find((s) => s.key === key);
        if (setting) {
          setting.value = setting.default_value;
          setting.is_custom = false;
        }
        return json(route, 200, { status: 'reset' });
      }

      if (path === '/admin/secrets' && method === 'GET') {
        return json(route, 200, this.adminSecrets);
      }

      if (path === '/admin/secrets' && method === 'POST') {
        const provider = String(body?.provider ?? 'openai');
        const slotName = String(body?.name ?? 'default');
        const rawSecret = String(body?.secret ?? '').trim();
        const id = `sec_${this.nextSecretId++}`;
        const masked = rawSecret && rawSecret.length > 10 ? `${rawSecret.slice(0, 3)}...${rawSecret.slice(-4)}` : '••••••••';
        const newSec = {
          id,
          provider,
          name: slotName || 'default',
          status: 'active',
          key_version: 1,
          masked_preview: masked,
          last_tested_at: null,
          last_test_status: null,
          last_test_error: null,
          created_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
        };
        this.adminSecrets.push(newSec);
        this.adminAuditLogs.unshift({
          id: this.adminAuditLogs.length + 1,
          actor_id: OWNER,
          action: 'secret_created',
          target_type: 'secret',
          target_key: `${provider}:${newSec.name}`,
          outcome: 'success',
          detail: { masked_preview: masked },
          created_at: new Date().toISOString(),
        });
        return json(route, 200, newSec);
      }

      if (path.startsWith('/admin/secrets/') && path.endsWith('/test') && method === 'POST') {
        const id = path.slice('/admin/secrets/'.length, -'/test'.length);
        const sec = this.adminSecrets.find((s) => s.id === id);
        if (sec) {
          sec.last_tested_at = new Date().toISOString();
          sec.last_test_status = 'healthy';
        }
        return json(route, 200, { id, status: 'healthy', error: null });
      }

      if (path.startsWith('/admin/secrets/') && method === 'PUT') {
        const id = path.slice('/admin/secrets/'.length);
        const sec = this.adminSecrets.find((s) => s.id === id);
        if (!sec) return json(route, 404, { error: 'Secret not found', code: 404 });
        if (body?.secret) {
          const rawSecret = String(body.secret).trim();
          sec.masked_preview = rawSecret.length > 10 ? `${rawSecret.slice(0, 3)}...${rawSecret.slice(-4)}` : '••••••••';
        }
        if (body?.status) sec.status = String(body.status);
        sec.updated_at = new Date().toISOString();
        return json(route, 200, sec);
      }

      if (path.startsWith('/admin/secrets/') && method === 'DELETE') {
        const id = path.slice('/admin/secrets/'.length);
        this.adminSecrets = this.adminSecrets.filter((s) => s.id !== id);
        return json(route, 200, { status: 'deleted' });
      }

      if (path === '/admin/reload' && method === 'POST') {
        return json(route, 200, { status: 'reloaded', reloaded_at: new Date().toISOString() });
      }

      if (path.startsWith('/admin/audit-logs') && method === 'GET') {
        const limit = Number(url?.searchParams.get('limit') ?? 50);
        const offset = Number(url?.searchParams.get('offset') ?? 0);
        return json(route, 200, auditPage(this.adminAuditLogs, limit, offset));
      }
    }

    // --- account management ---
    if (path === '/account/passkeys' && method === 'GET') {
      return json(route, 200, this.passkeys);
    }
    if (path.startsWith('/account/passkeys/') && method === 'DELETE') {
      const id = path.split('/').pop()!;
      if (this.passkeys.length <= 1) {
        return json(route, 409, {
          error: 'That is your only passkey. Add another before removing this one.',
          code: 409,
        });
      }
      this.passkeys = this.passkeys.filter((k) => k.id !== id);
      return route.fulfill({ status: 204, body: '' });
    }
    if (path === '/account/identities' && method === 'GET') {
      return json(route, 200, {
        identities: [
          ...this.linkedMxids.map((user_id) => ({
            channel: 'matrix',
            user_id,
            display_name: null,
            linked_at: '2026-08-01T10:00:00Z',
          })),
          ...this.otherIdentities,
        ],
      });
    }
    if (path === '/account/identities/unlink' && method === 'POST') {
      const before = this.linkedMxids.length + this.otherIdentities.length;
      if (body?.channel === 'matrix') {
        this.linkedMxids = this.linkedMxids.filter((m) => m !== body?.user_id);
      } else {
        this.otherIdentities = this.otherIdentities.filter(
          (i) => !(i.channel === body?.channel && i.user_id === body?.user_id)
        );
      }
      const after = this.linkedMxids.length + this.otherIdentities.length;
      return json(route, 200, { unlinked: after < before });
    }
    if (path === '/link/inspect' && method === 'POST') {
      const preview = this.linkPreviews[String(body?.token)] ?? this.linkPreview;
      return json(route, 200, preview);
    }
    if (path === '/link/claim' && method === 'POST') {
      return json(route, 200, { outcome: body?.token === 'merge-token' ? 'merged' : this.claimOutcome });
    }
    if (path === '/link/signup/start' && method === 'POST') {
      return json(route, 200, {
        ceremony_id: 'ceremony_link_signup',
        options: { publicKey: registrationOptions(DISPLAY_NAME) },
      });
    }
    if (path === '/link/signup/finish' && method === 'POST') {
      this.signedIn = true;
      return json(route, 200, { account_id: OWNER, display_name: DISPLAY_NAME });
    }
    if (path === '/account/sessions' && method === 'GET') {
      return json(route, 200, this.sessions);
    }
    if (path === '/account/sessions/revoke-others' && method === 'POST') {
      const revoked = this.sessions.filter((x) => !x.current).length;
      this.sessions = this.sessions.filter((x) => x.current);
      return json(route, 200, revoked);
    }
    const sessionMatch = path.match(/^\/account\/sessions\/([^/]+)$/);
    if (sessionMatch && method === 'DELETE') {
      const id = decodeURIComponent(sessionMatch[1]);
      this.sessions = this.sessions.filter((x) => x.id !== id);
      return json(route, 200, true);
    }
    if (path === '/account/recovery-codes' && method === 'POST') {
      this.recoveryStatus = { ...this.recoveryStatus, codes_remaining: 10 };
      return json(route, 200, {
        codes: Array.from({ length: 10 }, (_, i) => `CODE${i}-ABCDE`),
      });
    }
    if (path === '/account/recovery-status' && method === 'GET') {
      return json(route, 200, this.recoveryStatus);
    }
    if (path === '/account/recovery/cancel' && method === 'POST') {
      this.recoveryStatus = { ...this.recoveryStatus, recovery_valid_after: null };
      return json(route, 200, 1);
    }
    if (path === '/account/security-events' && method === 'GET') {
      return json(route, 200, this.securityEvents);
    }

    // --- settings ---
    // Every module reads this via `useSettings()`, so it has to be mocked even
    // though no current spec changes it — an unmocked 404 here is silent to the
    // app (`useSettings` falls back to `DEFAULT_SETTINGS`) but still a real
    // failed network request, and `console --errors` checks are supposed to
    // catch exactly that.
    if (path === '/settings' && method === 'GET') {
      return json(route, 200, this.settings);
    }
    if (path === '/settings' && method === 'PUT') {
      this.settings = { timezone: String(body?.timezone ?? 'UTC'), currency: String(body?.currency ?? 'USD') };
      return json(route, 200, this.settings);
    }

    // --- infrastructure ---
    // `/health/live` and `/health/ready` are mocked separately below: they live
    // outside `/api/v1`, unlike everything else this block handles.
    if (path === '/events') {
      // Frames queued by `pushEvent` before the stream opened are delivered,
      // then the response ends. `EventSource` reconnects with backoff, so a
      // test that queues an event and waits will get it on the next connect —
      // deterministic, without holding a socket open for the run.
      //
      // This used to answer with an empty body and nothing else, which meant
      // every live-update path in the app went permanently untested and the UI
      // sat in "Reconnecting…" for the whole suite.
      const frames = this.pendingEvents.splice(0).map(
        (e) => `data: ${JSON.stringify(e)}\n\n`
      );
      return route.fulfill({
        status: 200,
        contentType: 'text/event-stream',
        body: frames.join(''),
      });
    }
    if (path === '/rooms') {
      return json(route, 200, this.rooms);
    }

    // --- todos ---
    if (path === '/todos' && method === 'GET') {
      const completedParam = url?.searchParams?.get('completed');
      let items = this.todos;
      if (completedParam === 'true') {
        items = items.filter((t) => t.completed);
      } else if (completedParam === 'false') {
        items = items.filter((t) => !t.completed);
      }
      return json(route, 200, {
        items,
        page: 1,
        limit: 100,
        total: items.length,
      });
    }

    if (path === '/todos' && method === 'POST') {
      const text = String(body?.text ?? '').trim();
      if (!text) return json(route, 400, { error: 'Task text required', code: 400 });
      if (text.length > 500) {
        return json(route, 400, { error: 'Task text exceeds 500 character limit', code: 400 });
      }

      const lead = body?.remind_before_minutes ?? null;
      const roomId = body?.room_id || 'web';
      if (lead !== null) {
        const bad = this.assertLead(route, lead);
        if (bad) return bad;
        if (!body?.due_at) {
          return json(route, 400, {
            error: 'remind_before_minutes requires a due_at to count back from',
            code: 400,
          });
        }
        if (!body?.remind_room_id && roomId === 'web') {
          return json(route, 400, {
            error:
              'remind_room_id is required for a reminder on a task that has no Matrix room',
            code: 400,
          });
        }
      }

      const todo = this.seedTodo({
        room_id: roomId,
        text,
        priority: body?.priority ?? 'medium',
        due_at: body?.due_at ?? null,
        remind_before_minutes: lead,
      });
      this.syncDueReminder(todo, body?.remind_room_id ?? null);
      return json(route, 200, todo);
    }

    const todoMatch = path.match(/^\/todos\/(\d+)$/);
    if (todoMatch) {
      const id = Number(todoMatch[1]);
      const todo = this.todos.find((t) => t.id === id);
      if (!todo) {
        return json(route, 404, { error: 'Task not found or access denied', code: 404 });
      }

      if (method === 'DELETE') {
        const linked = this.reminderForTodo(id);
        if (linked) linked.status = 'cancelled';
        this.todos = this.todos.filter((t) => t.id !== id);
        return route.fulfill({ status: 204, body: '' });
      }

      if (method === 'PATCH') {
        if (body && 'remind_before_minutes' in body && body.remind_before_minutes !== null) {
          const bad = this.assertLead(route, body.remind_before_minutes);
          if (bad) return bad;
        }
        if (body?.text !== undefined) todo.text = String(body.text);
        if (body?.priority !== undefined) todo.priority = String(body.priority);
        if (body?.completed !== undefined) todo.completed = Boolean(body.completed);
        // `undefined` (absent) leaves the field alone; `null` clears it.
        if (body && 'due_at' in body) todo.due_at = body.due_at ?? null;
        if (body && 'remind_before_minutes' in body) {
          todo.remind_before_minutes = body.remind_before_minutes ?? null;
        }

        if (todo.due_at && todo.remind_before_minutes && !todo.completed) {
          const room =
            body?.remind_room_id || this.reminderForTodo(id)?.room_id ||
            (todo.room_id !== 'web' ? todo.room_id : null);
          if (!room) {
            return json(route, 400, {
              error:
                'remind_room_id is required for a reminder on a task that has no Matrix room',
              code: 400,
            });
          }
        }
        this.syncDueReminder(todo, body?.remind_room_id ?? null);
        return json(route, 200, todo);
      }
    }

    // --- finance ---
    if (path === '/finance/categories' && method === 'GET') {
      return json(route, 200, this.categories);
    }
    if (path === '/finance/categories' && method === 'POST') {
      const name = String(body?.name ?? '').trim();
      if (!name || name.length > 64) {
        return json(route, 400, { error: 'Category name must be 1–64 characters', code: 400 });
      }
      if (this.categories.some((c) => c.name.toLowerCase() === name.toLowerCase())) {
        return json(route, 409, {
          error: 'You already have a category with that name',
          code: 409,
        });
      }
      return json(route, 200, this.seedCategory(name));
    }
    const categoryMatch = path.match(/^\/finance\/categories\/(\d+)$/);
    if (categoryMatch && method === 'DELETE') {
      const id = Number(categoryMatch[1]);
      if (!this.categories.some((c) => c.id === id)) {
        return json(route, 404, { error: 'Category not found', code: 404 });
      }
      // Entries are kept and left uncategorised — money is never destroyed by
      // tidying the category list.
      let uncategorised = 0;
      for (const tx of this.transactions) {
        if (tx.category_id === id) {
          tx.category_id = null;
          tx.category_name = null;
          uncategorised += 1;
        }
      }
      this.categories = this.categories.filter((c) => c.id !== id);
      return json(route, 200, { id, uncategorised });
    }

    if (path === '/finance/transactions' && method === 'GET') {
      const direction = url?.searchParams?.get('direction');
      const items = direction
        ? this.transactions.filter((t) => t.direction === direction)
        : this.transactions;
      return json(route, 200, {
        items,
        page: 1,
        limit: 200,
        total: items.length,
      });
    }
    if (path === '/finance/transactions' && method === 'POST') {
      const parsed = parseAmount(String(body?.amount ?? ''), String(body?.currency ?? ''));
      if ('error' in parsed) return json(route, 400, { error: parsed.error, code: 400 });

      const categoryId = body?.category_id ?? null;
      const category = this.categories.find((c) => c.id === categoryId) ?? null;
      if (categoryId !== null && !category) {
        return json(route, 400, { error: 'Unknown category', code: 400 });
      }

      const tx = this.seedTransaction({
        direction: body?.direction ?? 'expense',
        amount_minor: parsed.minor,
        currency: parsed.currency,
        currency_exponent: parsed.exponent,
        category_id: category?.id ?? null,
        category_name: category?.name ?? null,
        note: body?.note ?? null,
        occurred_at: body?.occurred_at ?? new Date().toISOString(),
      });
      return json(route, 200, tx);
    }
    const txMatch = path.match(/^\/finance\/transactions\/(\d+)$/);
    if (txMatch && method === 'PUT') {
      const id = Number(txMatch[1]);
      const tx = this.transactions.find((t) => t.id === id);
      if (!tx) return json(route, 404, { error: 'Entry not found', code: 404 });

      const parsed = parseAmount(String(body?.amount ?? ''), String(body?.currency ?? ''));
      if ('error' in parsed) return json(route, 400, { error: parsed.error, code: 400 });

      const categoryId = body?.category_id ?? null;
      const category = this.categories.find((c) => c.id === categoryId) ?? null;
      if (categoryId !== null && !category) {
        return json(route, 400, { error: 'Unknown category', code: 400 });
      }

      tx.direction = body?.direction ?? tx.direction;
      tx.amount_minor = parsed.minor;
      tx.currency = parsed.currency;
      tx.currency_exponent = parsed.exponent;
      tx.category_id = category?.id ?? null;
      tx.category_name = category?.name ?? null;
      tx.note = body?.note ?? null;
      return json(route, 200, tx);
    }
    if (txMatch && method === 'DELETE') {
      const id = Number(txMatch[1]);
      if (!this.transactions.some((t) => t.id === id)) {
        return json(route, 404, { error: 'Entry not found', code: 404 });
      }
      this.transactions = this.transactions.filter((t) => t.id !== id);
      return route.fulfill({ status: 204, body: '' });
    }

    if (path === '/finance/net-worth' && method === 'GET') {
      const primaryCurrency = this.settings.currency || 'USD';
      const primaryExponent = primaryCurrency === 'JPY' ? 0 : 2;

      let totalLedgerMinor = 0;
      for (const tx of this.transactions) {
        if (tx.direction === 'income') totalLedgerMinor += tx.amount_minor;
        else totalLedgerMinor -= tx.amount_minor;
      }

      let totalPortfolioCashMinor = 0;
      for (const c of this.cash) {
        totalPortfolioCashMinor += c.balance_minor;
      }

      let totalMarketValueMinor = 0;
      for (const h of this.holdings) {
        const p = h.current_price_minor ?? h.avg_cost_minor;
        totalMarketValueMinor += Math.round((h.quantity_minor * p) / 10 ** 8);
      }

      const totalNetWorth = totalLedgerMinor + totalPortfolioCashMinor + totalMarketValueMinor;

      // Shape matches the real server response
      // (backend/src/modules/finance/net_worth.rs::NetWorthResponse /
      // CurrencyNetWorthBreakdown), not the earlier hand-written frontend
      // type this mock used to mirror.
      return json(route, 200, {
        primary_currency: primaryCurrency,
        primary_currency_exponent: primaryExponent,
        total_net_worth_minor: totalNetWorth,
        ledger_cash_minor: totalLedgerMinor,
        portfolio_market_value_minor: totalMarketValueMinor,
        cash_deposits_minor: totalPortfolioCashMinor,
        fx_rates: { [primaryCurrency]: 1.0 },
        updated_at: new Date().toISOString(),
        currencies: [
          {
            currency: primaryCurrency,
            currency_exponent: primaryExponent,
            ledger_cash_minor: totalLedgerMinor,
            portfolio_market_value_minor: totalMarketValueMinor,
            cash_deposits_minor: totalPortfolioCashMinor,
            fx_rate_to_primary: 1.0,
            converted_minor: totalNetWorth,
            total_minor: totalNetWorth,
          },
        ],
      });
    }

    if (path === '/finance/summary') {
      const byCurrency = new Map<string, any>();
      for (const tx of this.transactions) {
        if (!byCurrency.has(tx.currency)) {
          byCurrency.set(tx.currency, {
            currency: tx.currency,
            currency_exponent: tx.currency_exponent,
            expense_minor: 0,
            income_minor: 0,
            net_minor: 0,
            categories: [],
          });
        }
        const entry = byCurrency.get(tx.currency);
        if (tx.direction === 'expense') entry.expense_minor += tx.amount_minor;
        else entry.income_minor += tx.amount_minor;
        entry.net_minor = entry.income_minor - entry.expense_minor;

        let bucket = entry.categories.find((c: any) => c.category === tx.category_name);
        if (!bucket) {
          bucket = { category: tx.category_name, expense_minor: 0, income_minor: 0 };
          entry.categories.push(bucket);
        }
        if (tx.direction === 'expense') bucket.expense_minor += tx.amount_minor;
        else bucket.income_minor += tx.amount_minor;
      }
      return json(route, 200, [...byCurrency.values()]);
    }

    if (path === '/finance/analytics' && method === 'GET') {
      const currency = url?.searchParams?.get('currency') || 'USD';
      const months = Number(url?.searchParams?.get('months') || '6');
      const filtered = this.transactions.filter((t) => !currency || t.currency === currency);
      const totalIncome = filtered.filter((t) => t.direction === 'income').reduce((s, t) => s + t.amount_minor, 0);
      const totalExpense = filtered.filter((t) => t.direction === 'expense').reduce((s, t) => s + t.amount_minor, 0);

      const catMap = new Map<string, { amount_minor: number; tx_count: number }>();
      for (const t of filtered.filter((tx) => tx.direction === 'expense')) {
        const name = t.category_name || 'Uncategorised';
        const cur = catMap.get(name) || { amount_minor: 0, tx_count: 0 };
        cur.amount_minor += t.amount_minor;
        cur.tx_count += 1;
        catMap.set(name, cur);
      }
      const expense_categories = [...catMap.entries()].map(([category_name, v]) => ({
        category_name,
        amount_minor: v.amount_minor,
        percentage: totalExpense > 0 ? (v.amount_minor / totalExpense) * 100 : 0,
        tx_count: v.tx_count,
        currency,
        currency_exponent: 2,
      }));

      return json(route, 200, {
        currency,
        currency_exponent: 2,
        months_requested: months,
        total_income_minor: totalIncome,
        total_expense_minor: totalExpense,
        net_savings_minor: totalIncome - totalExpense,
        monthly_cashflow: [
          {
            month: '2026-08',
            income_minor: totalIncome,
            expense_minor: totalExpense,
            net_minor: totalIncome - totalExpense,
            currency,
            currency_exponent: 2,
          },
        ],
        expense_categories,
      });
    }

    if (path === '/finance/share' && method === 'GET') {
      return json(route, 200, this.financeShare);
    }
    if (path === '/finance/share' && method === 'POST') {
      this.financeShare = {
        owner_id: 'alice',
        token: this.financeShare?.token || 'fake_share_token_123',
        is_enabled: body?.is_enabled ?? true,
        title: body?.title ?? 'Public Fiscal Report',
        created_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
      };
      return json(route, 200, this.financeShare);
    }
    if (path === '/finance/share' && method === 'DELETE') {
      if (this.financeShare) this.financeShare.is_enabled = false;
      return json(route, 200, { success: true });
    }

    const pubMatch = path.match(/^\/public\/finance\/([^/]+)$/);
    if (pubMatch && method === 'GET') {
      const token = pubMatch[1];
      if (!this.financeShare || this.financeShare.token !== token || !this.financeShare.is_enabled) {
        return json(route, 404, { error: 'Report not found or link has expired', code: 404 });
      }
      return json(route, 200, {
        title: this.financeShare.title || 'Public Fiscal Report',
        currency: 'USD',
        currency_exponent: 2,
        total_income_minor: 10000,
        total_expense_minor: 5000,
        net_savings_minor: 5000,
        monthly_cashflow: [
          {
            month: '2026-08',
            income_minor: 10000,
            expense_minor: 5000,
            net_minor: 5000,
            currency: 'USD',
            currency_exponent: 2,
          },
        ],
        expense_categories: [
          {
            category_name: 'Food',
            amount_minor: 5000,
            percentage: 100,
            tx_count: 1,
            currency: 'USD',
            currency_exponent: 2,
          },
        ],
        recent_transactions: [],
      });
    }

    // --- reminders ---
    if (path === '/reminders' && method === 'GET') {
      const statusParam = url.searchParams.get('status') || 'pending';
      const page = parseInt(url.searchParams.get('page') || '1', 10);
      const limit = parseInt(url.searchParams.get('limit') || '100', 10);

      let filtered = this.reminders;
      if (statusParam === 'history') {
        filtered = this.reminders.filter((r) => r.status !== 'pending');
      } else if (statusParam === 'triggered' || statusParam === 'cancelled' || statusParam === 'failed') {
        filtered = this.reminders.filter((r) => r.status === statusParam);
      } else if (statusParam === 'pending') {
        filtered = this.reminders.filter((r) => r.status === 'pending');
      }

      const total = filtered.length;
      const offset = (page - 1) * limit;
      const paged = filtered.slice(offset, offset + limit);

      return json(route, 200, {
        items: paged,
        page,
        limit,
        total,
        has_more: offset + paged.length < total,
      });
    }
    if (path === '/reminders' && method === 'POST') {
      const message = String(body?.message ?? '').trim();
      if (!message) return json(route, 400, { error: 'Reminder message required', code: 400 });
      const reminder = this.seedReminder({
        message,
        room_id: body?.room_id || 'web',
        trigger_at: body?.trigger_at ?? new Date(Date.now() + 3_600_000).toISOString(),
        recurrence: body?.recurrence ?? 'none',
      });
      return json(route, 200, reminder);
    }
    if (path === '/reminders/history' && method === 'DELETE') {
      this.reminders = this.reminders.filter((r) => r.status === 'pending');
      return route.fulfill({ status: 204, body: '' });
    }
    const reminderMatch = path.match(/^\/reminders\/(\d+)$/);
    if (reminderMatch && method === 'DELETE') {
      const id = Number(reminderMatch[1]);
      const reminderIndex = this.reminders.findIndex((r) => r.id === id);
      if (reminderIndex === -1) return json(route, 404, { error: 'Reminder not found', code: 404 });
      const reminder = this.reminders[reminderIndex];
      if (reminder.status === 'pending') {
        reminder.status = 'cancelled';
      } else {
        this.reminders.splice(reminderIndex, 1);
      }
      return route.fulfill({ status: 204, body: '' });
    }

    // --- portfolio ---
    if (path === '/portfolio/holdings' && method === 'GET') {
      return json(route, 200, this.holdings);
    }
    if (path === '/portfolio/holdings' && method === 'POST') {
      const symbol = String(body?.symbol ?? '').trim().toUpperCase();
      if (!symbol || symbol.length > 12) {
        return json(route, 400, { error: 'Symbol must be 1–12 characters', code: 400 });
      }
      const currency = String(body?.currency ?? '').trim().toUpperCase();
      if (!/^[A-Z]{3}$/.test(currency)) {
        return json(route, 400, { error: 'Currency must be a 3-letter code such as EUR or JPY', code: 400 });
      }
      const exponent = exponentFor(currency);
      const price = parseDecimal(String(body?.price ?? ''), exponent);
      if ('error' in price) return json(route, 400, { error: price.error, code: 400 });
      const quantity = parseDecimal(String(body?.quantity ?? ''), QUANTITY_EXPONENT);
      if ('error' in quantity) return json(route, 400, { error: quantity.error, code: 400 });

      const existing = this.holdings.find((h) => h.symbol === symbol);
      const note = typeof body?.note === 'string' && body.note.trim() ? body.note : null;

      if (existing && existing.currency !== currency) {
        return json(route, 400, {
          error: `${symbol} is already held in ${existing.currency}`,
          code: 400,
        });
      }

      // Buying requires and debits sufficient cash — mirrors `PortfolioRepository::buy`,
      // which gates the whole write on this before touching the holding at all.
      const needed = Math.trunc((quantity.minor * price.minor) / 10 ** QUANTITY_EXPONENT);
      const cashRow = this.cash.find((c) => c.currency === currency);
      const available = cashRow?.balance_minor ?? 0;
      if (needed > available) {
        return json(route, 400, {
          error: `Need ${formatDecimal(needed, exponent)} ${currency} but only ${formatDecimal(available, exponent)} ${currency} in cash`,
          code: 400,
        });
      }
      this.seedCash(currency, -needed, exponent);

      if (!existing) {
        const holding = this.seedHolding({
          symbol,
          quantity_minor: quantity.minor,
          avg_cost_minor: price.minor,
          currency,
          currency_exponent: exponent,
          note,
        });
        return json(route, 200, holding);
      }

      const newQty = existing.quantity_minor + quantity.minor;
      existing.avg_cost_minor = Math.trunc(
        (existing.quantity_minor * existing.avg_cost_minor + quantity.minor * price.minor) / newQty
      );
      existing.quantity_minor = newQty;
      if (note) existing.note = note;
      existing.updated_at = new Date().toISOString();
      return json(route, 200, existing);
    }

    const holdingSellMatch = path.match(/^\/portfolio\/holdings\/(\d+)\/sell$/);
    if (holdingSellMatch && method === 'POST') {
      const id = Number(holdingSellMatch[1]);
      const holding = this.holdings.find((h) => h.id === id);
      if (!holding) return json(route, 404, { error: 'Holding not found or access denied', code: 404 });

      const price = parseDecimal(String(body?.price ?? ''), holding.currency_exponent);
      if ('error' in price) return json(route, 400, { error: price.error, code: 400 });
      const quantity = parseDecimal(String(body?.quantity ?? ''), QUANTITY_EXPONENT);
      if ('error' in quantity) return json(route, 400, { error: quantity.error, code: 400 });

      if (quantity.minor > holding.quantity_minor) {
        return json(route, 400, { error: `Only ${holding.quantity_minor} held (minor units)`, code: 400 });
      }

      const realized = Math.trunc(
        (quantity.minor * (price.minor - holding.avg_cost_minor)) / 10 ** QUANTITY_EXPONENT
      );
      // Cash is credited the full gross proceeds, not just the (possibly
      // negative) realized P&L — a losing sale still returns the principal.
      const proceeds = Math.trunc((quantity.minor * price.minor) / 10 ** QUANTITY_EXPONENT);
      this.seedCash(holding.currency, proceeds, holding.currency_exponent);

      const remaining = holding.quantity_minor - quantity.minor;

      if (remaining === 0) {
        this.holdings = this.holdings.filter((h) => h.id !== id);
        return json(route, 200, { holding: null, realized_pl_minor: realized });
      }

      holding.quantity_minor = remaining;
      holding.updated_at = new Date().toISOString();
      return json(route, 200, { holding, realized_pl_minor: realized });
    }

    const holdingPriceMatch = path.match(/^\/portfolio\/holdings\/(\d+)\/price$/);
    if (holdingPriceMatch && method === 'PUT') {
      const id = Number(holdingPriceMatch[1]);
      const holding = this.holdings.find((h) => h.id === id);
      if (!holding) return json(route, 404, { error: 'Holding not found or access denied', code: 404 });

      const price = parseDecimal(String(body?.price ?? ''), holding.currency_exponent);
      if ('error' in price) return json(route, 400, { error: price.error, code: 400 });

      holding.current_price_minor = price.minor;
      holding.price_updated_at = new Date().toISOString();
      holding.updated_at = new Date().toISOString();
      return json(route, 200, holding);
    }

    const holdingNoteMatch = path.match(/^\/portfolio\/holdings\/(\d+)\/note$/);
    if (holdingNoteMatch && method === 'PUT') {
      const id = Number(holdingNoteMatch[1]);
      const holding = this.holdings.find((h) => h.id === id);
      if (!holding) return json(route, 404, { error: 'Holding not found or access denied', code: 404 });

      const note = typeof body?.note === 'string' && body.note.trim() ? body.note : null;
      holding.note = note;
      holding.updated_at = new Date().toISOString();
      return json(route, 200, holding);
    }

    const holdingMatch = path.match(/^\/portfolio\/holdings\/(\d+)$/);
    if (holdingMatch && method === 'DELETE') {
      const id = Number(holdingMatch[1]);
      if (!this.holdings.some((h) => h.id === id)) {
        return json(route, 404, { error: 'Holding not found or access denied', code: 404 });
      }
      this.holdings = this.holdings.filter((h) => h.id !== id);
      return route.fulfill({ status: 204, body: '' });
    }

    // --- portfolio cash ---
    if (path === '/portfolio/cash' && method === 'GET') {
      return json(route, 200, this.cash);
    }
    if (path === '/portfolio/cash/deposit' && method === 'POST') {
      const currency = String(body?.currency ?? '').trim().toUpperCase();
      if (!/^[A-Z]{3}$/.test(currency)) {
        return json(route, 400, { error: 'Currency must be a 3-letter code such as EUR or JPY', code: 400 });
      }
      const exponent = exponentFor(currency);
      const amount = parseDecimal(String(body?.amount ?? ''), exponent);
      if ('error' in amount) return json(route, 400, { error: amount.error, code: 400 });

      const cash = this.seedCash(currency, amount.minor, exponent);
      this.seedTransaction({
        direction: 'expense',
        amount_minor: amount.minor,
        currency,
        currency_exponent: exponent,
        category_name: 'Portfolio Cash Deposit',
        note: `Portfolio cash deposit (${currency})`,
        source_kind: 'portfolio_cash',
      });
      return json(route, 200, cash);
    }
    if (path === '/portfolio/cash/withdraw' && method === 'POST') {
      const currency = String(body?.currency ?? '').trim().toUpperCase();
      if (!/^[A-Z]{3}$/.test(currency)) {
        return json(route, 400, { error: 'Currency must be a 3-letter code such as EUR or JPY', code: 400 });
      }
      const exponent = exponentFor(currency);
      const amount = parseDecimal(String(body?.amount ?? ''), exponent);
      if ('error' in amount) return json(route, 400, { error: amount.error, code: 400 });

      const existing = this.cash.find((c) => c.currency === currency);
      const available = existing?.balance_minor ?? 0;
      if (amount.minor > available) {
        return json(route, 400, {
          error: `Only ${formatDecimal(available, exponent)} ${currency} available`,
          code: 400,
        });
      }
      const cash = this.seedCash(currency, -amount.minor, exponent);
      this.seedTransaction({
        direction: 'income',
        amount_minor: amount.minor,
        currency,
        currency_exponent: exponent,
        category_name: 'Portfolio Cash Withdrawal',
        note: `Portfolio cash withdrawal (${currency})`,
        source_kind: 'portfolio_cash',
      });
      return json(route, 200, cash);
    }

    if (path === '/portfolio/summary') {
      const byCurrency = new Map<string, any>();
      const entryFor = (currency: string, exponent: number) => {
        if (!byCurrency.has(currency)) {
          byCurrency.set(currency, {
            currency,
            currency_exponent: exponent,
            cost_basis_minor: 0,
            market_value_minor: 0,
            unrealized_pl_minor: 0,
            priced_holdings: 0,
            unpriced_holdings: 0,
            cash_minor: 0,
          });
        }
        return byCurrency.get(currency);
      };

      for (const h of this.holdings) {
        const entry = entryFor(h.currency, h.currency_exponent);
        const costBasis = Math.trunc((h.quantity_minor * h.avg_cost_minor) / 10 ** QUANTITY_EXPONENT);
        entry.cost_basis_minor += costBasis;
        if (h.current_price_minor != null) {
          const marketValue = Math.trunc(
            (h.quantity_minor * h.current_price_minor) / 10 ** QUANTITY_EXPONENT
          );
          entry.market_value_minor += marketValue;
          entry.unrealized_pl_minor += marketValue - costBasis;
          entry.priced_holdings += 1;
        } else {
          entry.unpriced_holdings += 1;
        }
      }
      // A currency seeded only by cash (a deposit with nothing bought yet)
      // must still appear.
      for (const c of this.cash) {
        entryFor(c.currency, c.currency_exponent).cash_minor = c.balance_minor;
      }

      return json(route, 200, [...byCurrency.values()]);
    }

    // --- portfolio alerts ---
    if (path === '/portfolio/alerts' && method === 'GET') {
      return json(route, 200, this.alerts);
    }
    if (path === '/portfolio/alerts' && method === 'POST') {
      const symbol = String(body?.symbol ?? '').trim().toUpperCase();
      const condition = body?.condition === 'below' ? 'below' : 'above';
      const target_price = String(body?.target_price ?? '').trim();
      const currency = String(body?.currency ?? 'USD').trim().toUpperCase();
      const exponent = exponentFor(currency);
      const parsed = parseDecimal(target_price, exponent);
      const priceMinor = 'minor' in parsed ? parsed.minor : 10000;
      const alert = {
        id: ++this.nextHoldingId,
        owner_id: OWNER,
        symbol,
        target_price_minor: priceMinor,
        currency,
        currency_exponent: exponent,
        condition,
        is_triggered: false,
        triggered_at: null,
        created_at: new Date().toISOString(),
      };
      this.alerts.push(alert);
      return json(route, 201, alert);
    }
    const alertDeleteMatch = path.match(/^\/portfolio\/alerts\/(\d+)$/);
    if (alertDeleteMatch && method === 'DELETE') {
      const id = Number(alertDeleteMatch[1]);
      this.alerts = this.alerts.filter((a) => a.id !== id);
      return route.fulfill({ status: 204, body: '' });
    }

    // --- portfolio history ---
    if (path.startsWith('/portfolio/history') && method === 'GET') {
      const urlObj = new URL('http://dummy' + path);
      const timeframe = urlObj.searchParams.get('timeframe') || '1m';
      const numDays = timeframe === '7d' ? 7 : timeframe === '3m' ? 90 : timeframe === '1y' ? 365 : 30;
      const totalCash = this.cash.reduce((acc, c) => acc + c.balance_minor, 0);
      const totalHoldings = this.holdings.reduce((acc, h) => {
        const p = h.current_price_minor ?? h.avg_cost_minor;
        return acc + Math.trunc((h.quantity_minor * p) / 10 ** QUANTITY_EXPONENT);
      }, 0);

      const points = [];
      const now = new Date();
      for (let i = numDays - 1; i >= 0; i--) {
        const d = new Date(now.getTime() - i * 86400000);
        const dateStr = d.toISOString().split('T')[0];
        const factor = 1.0 - (i / numDays) * 0.05 + Math.sin(i) * 0.01;
        const hVal = Math.trunc(totalHoldings * factor);
        points.push({
          date: dateStr,
          holdings_value_minor: hVal,
          cash_minor: totalCash,
          total_net_worth_minor: hVal + totalCash,
        });
      }

      const starting = points[0].total_net_worth_minor;
      const current = points[points.length - 1].total_net_worth_minor;
      const change = current - starting;
      const pct = starting > 0 ? (change / starting) * 100 : 0;

      return json(route, 200, {
        currency: 'USD',
        currency_exponent: 2,
        timeframe,
        points,
        starting_value_minor: starting,
        current_value_minor: current,
        period_change_minor: change,
        period_change_pct: pct,
      });
    }

    // --- webhooks ---
    if (path === '/account/webhooks' && method === 'GET') {
      return json(route, 200, this.webhooks);
    }
    if (path === '/account/webhooks' && method === 'POST') {
      const name = String(body?.name ?? '').trim();
      const roomId = String(body?.default_room_id ?? '').trim();
      if (!name) return json(route, 400, { error: 'Webhook name is required', code: 400 });
      if (!roomId) return json(route, 400, { error: 'Target room ID cannot be empty', code: 400 });
      const webhook = this.seedWebhook({ name, default_room_id: roomId });
      const rawToken = 'whk_fake_secret_token_1234567890abcdef';
      return json(route, 201, {
        webhook,
        raw_token: rawToken,
        example_curl_json: `curl -X POST https://localhost:8080/api/v1/webhooks/${rawToken} -H "Content-Type: application/json" -d '{"title":"Alert","message":"Server is down","level":"critical"}'`,
        example_curl_telegram: `curl -X POST https://localhost:8080/bot${rawToken}/sendMessage -d "text=Server alert"`,
      });
    }
    const webhookMatch = path.match(/^\/account\/webhooks\/([^/]+)$/);
    if (webhookMatch) {
      const id = webhookMatch[1];
      const hook = this.webhooks.find((w) => w.id === id);
      if (!hook) return json(route, 404, { error: 'Webhook not found', code: 404 });

      if (method === 'PATCH') {
        if (body?.name !== undefined) hook.name = String(body.name);
        if (body?.default_room_id !== undefined) hook.default_room_id = String(body.default_room_id);
        if (body?.is_active !== undefined) hook.is_active = Boolean(body.is_active);
        return json(route, 200, hook);
      }
      if (method === 'DELETE') {
        this.webhooks = this.webhooks.filter((w) => w.id !== id);
        return route.fulfill({ status: 204, body: '' });
      }
    }
    const rotateMatch = path.match(/^\/account\/webhooks\/([^/]+)\/rotate$/);
    if (rotateMatch && method === 'POST') {
      const id = rotateMatch[1];
      const hook = this.webhooks.find((w) => w.id === id);
      if (!hook) return json(route, 404, { error: 'Webhook not found', code: 404 });
      const rawToken = 'whk_rotated_secret_token_9876543210fedcba';
      return json(route, 200, {
        webhook: hook,
        raw_token: rawToken,
        example_curl_json: `curl -X POST https://localhost:8080/api/v1/webhooks/${rawToken} -H "Content-Type: application/json" -d '{"title":"Alert","message":"Server is down","level":"critical"}'`,
        example_curl_telegram: `curl -X POST https://localhost:8080/bot${rawToken}/sendMessage -d "text=Server alert"`,
      });
    }
    const testHookMatch = path.match(/^\/account\/webhooks\/([^/]+)\/test$/);
    if (testHookMatch && method === 'POST') {
      const id = testHookMatch[1];
      const hook = this.webhooks.find((w) => w.id === id);
      if (!hook) return json(route, 404, { error: 'Webhook not found', code: 404 });
      hook.total_delivered_count += 1;
      hook.last_triggered_at = new Date().toISOString();
      this.seedWebhookLog(id, {
        title: 'Test Webhook Alert',
        plain_body: 'This is a synthetic test alert dispatched from the Web UI.',
        html_body: '<b>Test Webhook Alert</b><br/>This is a synthetic test alert dispatched from the Web UI.',
        severity: 'info',
        status: 'delivered',
      });
      return json(route, 202, { status: 'queued', outbox_id: 'outbox_test_123' });
    }

    const logsMatch = path.match(/^\/account\/webhooks\/([^/]+)\/logs$/);
    if (logsMatch && method === 'GET') {
      const id = logsMatch[1];
      const items = this.webhookLogs.filter((l) => l.webhook_id === id);
      return json(route, 200, {
        items,
        total: items.length,
        limit: 50,
        offset: 0,
      });
    }

    if (path === '/account/ai-config') {
      if (method === 'GET') {
        const redacted = {
          ...this.aiConfig,
          has_api_key: !!this.aiConfig.api_key && this.aiConfig.api_key.trim().length > 0,
          api_key: null,
        };
        return json(route, 200, redacted);
      }
      if (method === 'PUT') {
        this.aiConfig = {
          ...this.aiConfig,
          ...body,
          updated_at: new Date().toISOString(),
        };
        const redacted = {
          ...this.aiConfig,
          has_api_key: !!this.aiConfig.api_key && this.aiConfig.api_key.trim().length > 0,
          api_key: null,
        };
        return json(route, 200, redacted);
      }
    }

    if (path === '/ai/verify' && method === 'POST') {
      const apiKey = body?.api_key || this.aiConfig.api_key;
      const baseUrl = body?.api_base_url || this.aiConfig.api_base_url;
      if (!apiKey && baseUrl && !baseUrl.includes('localhost') && !baseUrl.includes('127.0.0.1')) {
        return json(route, 200, {
          valid: false,
          message: 'No API key provided to verify. Please enter an API key.',
        });
      }
      if (apiKey === 'invalid_key') {
        return json(route, 200, {
          valid: false,
          message: 'Invalid API key (HTTP 401): Unauthorized',
        });
      }
      return json(route, 200, {
        valid: true,
        message: 'Connection successful and API key verified!',
      });
    }

    if (path === '/ai/chat' && method === 'POST') {
      let conv = this.aiConversations.find((c) => c.id === body?.conversation_id);
      if (!conv) {
        conv = {
          id: `conv_${this.nextAiConvId++}`,
          owner_id: OWNER,
          room_id: String(body?.room_id || '!room1:example.com'),
          created_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
        };
        this.aiConversations.push(conv);
      }
      conv.updated_at = new Date().toISOString();

      const userMsg: FakeAiMessage = {
        id: `msg_${this.nextAiMsgId++}`,
        conversation_id: conv.id,
        role: 'user',
        content: String(body?.prompt || ''),
        tool_call_id: null,
        tool_name: null,
        created_at: new Date().toISOString(),
      };
      this.aiMessages.push(userMsg);

      const lowerPrompt = (body?.prompt || '').toLowerCase();
      const isMultiAction = lowerPrompt.includes('shake shack') || (lowerPrompt.includes('spent') && (lowerPrompt.includes('habit') || lowerPrompt.includes('remind')));
      const isRetroPrompt = lowerPrompt.includes('retro') || lowerPrompt.includes('retrospective');
      const isTaskQuestion = !isMultiAction && !isRetroPrompt && lowerPrompt.includes('task');
      const isPortfolioPrompt = !isMultiAction && !isRetroPrompt && !isTaskQuestion && lowerPrompt.includes('portfolio');

      const toolCalls: string[] = [];
      let botReply = `I received your prompt: "${body?.prompt}". How can I help further?`;

      if (isMultiAction) {
        toolCalls.push('record_finance_entry', 'log_habit', 'set_reminder');
        this.aiMessages.push({
          id: `msg_${this.nextAiMsgId++}`,
          conversation_id: conv.id,
          role: 'tool',
          content: JSON.stringify({
            tools_executed: [
              { tool: 'record_finance_entry', status: 'success', amount: 16.50, category: 'Dining' },
              { tool: 'log_habit', status: 'success', habit: 'Workout', completed: true },
              { tool: 'set_reminder', status: 'success', message: 'Call mom', trigger_at: '19:00' },
            ],
          }),
          tool_call_id: 'call_multi',
          tool_name: 'multi_action_executor',
          created_at: new Date().toISOString(),
        });
        botReply = "I've handled all three actions for you:\n1. 💸 Recorded $16.50 for Dining at Shake Shack\n2. ✅ Marked Workout habit as completed\n3. ⏰ Scheduled reminder to call mom at 7:00 PM";
      } else if (isRetroPrompt) {
        toolCalls.push('get_weekly_retrospective');
        botReply = "### 🌟 Weekly Executive Retrospective (2026-08-29 to 2026-09-05)\n\n**💰 Financial Velocity**\n- **Total Spent**: `USD 125.50`\n- **Total Income**: `USD 2500.00`\n- **Net Cashflow**: `USD 2374.50`\n\n**📋 Productivity & Habits**\n- **Tasks Completed**: 8\n- **Habit Adherence**: 85.7%\n\n**🧬 Life & Wellness**\n- **Days Logged**: 6 of 7 days\n- **Dominant Mood**: Good";
      } else if (isPortfolioPrompt) {
        toolCalls.push('get_my_portfolio_summary');
        this.aiMessages.push({
          id: `msg_${this.nextAiMsgId++}`,
          conversation_id: conv.id,
          role: 'tool',
          content: JSON.stringify([
            { currency: 'USD', cost_basis_minor: 650000, market_value_minor: 799816, cash_minor: 0 },
          ]),
          tool_call_id: 'call_port',
          tool_name: 'get_my_portfolio_summary',
          created_at: new Date().toISOString(),
        });
        botReply = "**Portfolio & cash breakdown**\n\n### USD\n| Asset | Value |\n|---|---|\n| **Cash** | $0.00 |\n| **Holdings** | 1 priced position |\n| **Cost basis** | $6,500.00 |\n| **Market value** | $7,998.16 |\n| **Unrealized P&L** | **+$1,498.16** |";
      } else if (isTaskQuestion) {
        toolCalls.push('get_my_tasks');
        this.aiMessages.push({
          id: `msg_${this.nextAiMsgId++}`,
          conversation_id: conv.id,
          role: 'tool',
          content: JSON.stringify(
            this.todos.map((t) => ({
              id: t.id,
              text: t.text,
              priority: t.priority,
              completed: t.completed,
            })),
            null,
            2
          ),
          tool_call_id: 'call_123',
          tool_name: 'get_my_tasks',
          created_at: new Date().toISOString(),
        });
        botReply = `You have ${this.todos.length} task(s) currently recorded in your list.`;
      }

      const assistantMsg: FakeAiMessage = {
        id: `msg_${this.nextAiMsgId++}`,
        conversation_id: conv.id,
        role: 'assistant',
        content: botReply,
        tool_call_id: null,
        tool_name: null,
        created_at: new Date().toISOString(),
      };
      this.aiMessages.push(assistantMsg);

      return json(route, 200, {
        conversation_id: conv.id,
        message: botReply,
        model_used: this.aiConfig.model_name || 'gpt-4o-mini',
        tool_calls_executed: toolCalls,
      });
    }

    if (path === '/ai/retrospective' && method === 'GET') {
      return json(route, 200, {
        period_start: '2026-08-29',
        period_end: '2026-09-05',
        total_spent_minor: 12550,
        total_income_minor: 250000,
        net_cashflow_minor: 237450,
        currency: 'USD',
        spends_by_currency: [
          {
            currency: 'USD',
            currency_exponent: 2,
            spent_minor: 12550,
            income_minor: 250000,
            net_minor: 237450,
          },
        ],
        upcoming_bills_by_currency: [
          {
            currency: 'USD',
            currency_exponent: 2,
            total_minor: 4500,
          },
        ],
        formatted_spent: 'USD 125.50',
        formatted_income: 'USD 2500.00',
        formatted_net: 'USD 2374.50',
        formatted_upcoming_bills: 'USD 45.00',
        top_expense_category: 'Dining',
        completed_tasks_count: 8,
        active_tasks_count: 3,
        habits_adherence_percent: 85.7,
        active_habits_count: 4,
        upcoming_bills_count: 2,
        upcoming_bills_total_minor: 4500,
        upcoming_reminders_count: 3,
        days_logged_count: 6,
        dominant_mood: 'good',
        formatted_markdown:
          '### 🌟 Weekly Executive Retrospective (2026-08-29 to 2026-09-05)\n\n' +
          '**💰 Financial Velocity**\n' +
          '- **Total Spent**: `USD 125.50`\n' +
          '- **Total Income**: `USD 2500.00`\n' +
          '- **Net Cashflow**: `USD 2374.50`\n' +
          '- **Top Category**: Dining\n\n' +
          '**📋 Productivity & Habits**\n' +
          '- **Tasks Completed**: 8 (Active remaining: 3)\n' +
          '- **Habit Adherence**: 85.7% across 4 tracked habits\n\n' +
          '**🧬 Life & Wellness**\n' +
          '- **Days Logged**: 6 of 7 days\n' +
          '- **Dominant Mood**: Good\n\n' +
          '**📅 Upcoming Commitments (Next 7 Days)**\n' +
          '- **Bills Due**: 2 (`USD 45.00`)\n' +
          '- **Pending Reminders**: 3',
      });
    }

    if (path === '/ai/conversations' && method === 'GET') {
      return json(route, 200, this.aiConversations);
    }

    const aiConvMatch = path.match(/^\/ai\/conversations\/([^/]+)$/);
    if (aiConvMatch && method === 'DELETE') {
      const id = aiConvMatch[1];
      this.aiConversations = this.aiConversations.filter((c) => c.id !== id);
      this.aiMessages = this.aiMessages.filter((m) => m.conversation_id !== id);
      return route.fulfill({ status: 204, body: '' });
    }

    const aiMsgMatch = path.match(/^\/ai\/conversations\/([^/]+)\/messages$/);
    if (aiMsgMatch && method === 'GET') {
      const id = aiMsgMatch[1];
      const msgs = this.aiMessages.filter((m) => m.conversation_id === id);
      return json(route, 200, msgs);
    }

    // --- footprints ---
    if (path === '/footprints' && method === 'GET') {
      return json(route, 200, this.footprints);
    }
    if (path === '/footprints/stats' && method === 'GET') {
      const countries = new Set(this.footprints.map((f) => f.country).filter(Boolean));
      const days = this.footprints.reduce((sum, f) => sum + f.stay_duration_days, 0);
      const byCurr = new Map<string, { currency: string; currency_exponent: number; total_spend_minor: number }>();
      for (const f of this.footprints) {
        if (f.spend_amount_minor > 0) {
          const cur = byCurr.get(f.spend_currency);
          if (cur) {
            cur.total_spend_minor += f.spend_amount_minor;
          } else {
            byCurr.set(f.spend_currency, {
              currency: f.spend_currency,
              currency_exponent: f.spend_currency_exponent,
              total_spend_minor: f.spend_amount_minor,
            });
          }
        }
      }
      return json(route, 200, {
        total_places: this.footprints.length,
        total_countries: countries.size,
        total_days_traveled: days,
        spend_by_currency: Array.from(byCurr.values()),
      });
    }
    if (path.startsWith('/footprints/geocode') && method === 'GET') {
      if (path.includes('lat=') && path.includes('lon=')) {
        return json(route, 200, {
          display_name: 'Colosseum, Rome, Italy',
          name: 'Colosseum',
          address: { city: 'Rome', country: 'Italy' },
        });
      }
      return json(route, 200, [
        {
          place_id: 1,
          lat: '48.8584',
          lon: '2.2945',
          display_name: 'Eiffel Tower, Paris, France',
          address: { city: 'Paris', country: 'France' },
        },
      ]);
    }
    if (path === '/footprints' && method === 'POST') {
      const title = String(body?.title ?? '').trim();
      if (!title) return json(route, 400, { error: 'Title required', code: 400 });
      const spendCurrency = String(body?.spend_currency ?? 'USD').toUpperCase();
      const exp = exponentFor(spendCurrency);
      const fp = this.seedFootprint({
        title,
        latitude: body?.latitude ?? 0,
        longitude: body?.longitude ?? 0,
        country: body?.country ?? null,
        city: body?.city ?? null,
        address: body?.address ?? null,
        arrival_date: body?.arrival_date ?? '2026-06-01',
        departure_date: body?.departure_date ?? null,
        spend_amount_minor: body?.spend_amount
          ? Math.round(parseFloat(body.spend_amount) * 10 ** exp)
          : 0,
        spend_currency: spendCurrency,
        spend_currency_exponent: exp,
        rating: body?.rating ?? 5,
        category: body?.category ?? 'sightseeing',
        note: body?.note ?? null,
      });
      return json(route, 200, fp);
    }
    const fpMatch = path.match(/^\/footprints\/(\d+)$/);
    if (fpMatch && method === 'GET') {
      const id = Number(fpMatch[1]);
      const fp = this.footprints.find((f) => f.id === id);
      if (!fp) return json(route, 404, { error: 'Footprint not found', code: 404 });
      return json(route, 200, fp);
    }
    if (fpMatch && method === 'PUT') {
      const id = Number(fpMatch[1]);
      const fp = this.footprints.find((f) => f.id === id);
      if (!fp) return json(route, 404, { error: 'Footprint not found', code: 404 });
      if (body?.title) fp.title = body.title;
      if (body?.country) fp.country = body.country;
      if (body?.city) fp.city = body.city;
      if (body?.note !== undefined) fp.note = body.note;
      if (body?.rating !== undefined) fp.rating = body.rating;
      if (body?.spend_currency) {
        fp.spend_currency = String(body.spend_currency).toUpperCase();
        fp.spend_currency_exponent = exponentFor(fp.spend_currency);
      }
      if (body?.spend_amount !== undefined) {
        fp.spend_amount_minor = body.spend_amount
          ? Math.round(parseFloat(body.spend_amount) * 10 ** fp.spend_currency_exponent)
          : 0;
      }
      fp.updated_at = new Date().toISOString();
      return json(route, 200, fp);
    }
    if (fpMatch && method === 'DELETE') {
      const id = Number(fpMatch[1]);
      const idx = this.footprints.findIndex((f) => f.id === id);
      if (idx === -1) return json(route, 404, { error: 'Footprint not found', code: 404 });
      this.footprints.splice(idx, 1);
      return json(route, 200, { success: true, id });
    }

    // --- habits ---
    if (path === '/habits' && method === 'GET') {
      const todayStr = new Date().toISOString().split('T')[0];
      const items = this.habits.map((h) => {
        const logs = this.habitLogs.filter((l) => l.habit_id === h.id && l.completed);
        const logDates = new Set(logs.map((l) => l.log_date));
        const completed_today = logDates.has(todayStr);

        let streak = 0;
        let cur = new Date();
        let checkStr = todayStr;
        if (!completed_today) {
          cur.setDate(cur.getDate() - 1);
          checkStr = cur.toISOString().split('T')[0];
        }
        while (logDates.has(checkStr)) {
          streak++;
          cur.setDate(cur.getDate() - 1);
          checkStr = cur.toISOString().split('T')[0];
        }

        const last_7_days: { date: string; completed: boolean }[] = [];
        for (let i = 6; i >= 0; i--) {
          const d = new Date();
          d.setDate(d.getDate() - i);
          const ds = d.toISOString().split('T')[0];
          last_7_days.push({ date: ds, completed: logDates.has(ds) });
        }

        return {
          ...h,
          completed_today,
          current_streak: streak,
          longest_streak: streak,
          total_completions: logs.length,
          last_7_days,
        };
      });
      return json(route, 200, items);
    }

    if (path === '/habits/stats' && method === 'GET') {
      const todayStr = new Date().toISOString().split('T')[0];
      const completedToday = this.habits.filter((h) =>
        this.habitLogs.some((l) => l.habit_id === h.id && l.completed && l.log_date === todayStr)
      ).length;
      return json(route, 200, {
        total_active_habits: this.habits.length,
        completed_today: completedToday,
        completion_rate_today_percent: this.habits.length > 0 ? Math.round((completedToday / this.habits.length) * 100) : 0,
        best_streak: 0,
        total_all_time_completions: this.habitLogs.filter((l) => l.completed).length,
      });
    }

    if (path === '/habits/heatmap' && method === 'GET') {
      const dateCounts = new Map<string, number>();
      for (const log of this.habitLogs) {
        if (log.completed) {
          dateCounts.set(log.log_date, (dateCounts.get(log.log_date) || 0) + 1);
        }
      }
      const days = Array.from(dateCounts.entries()).map(([date, count]) => ({ date, count }));
      return json(route, 200, days);
    }

    if (path === '/habits' && method === 'POST') {
      if (!body?.name?.trim()) {
        return json(route, 400, { error: 'Habit name is required.', code: 400 });
      }
      const habit = this.seedHabit({
        name: body.name.trim(),
        description: body.description ?? null,
        category: body.category || 'wellness',
        frequency: body.frequency || 'daily',
        target_days_per_week: body.target_days_per_week || 7,
      });
      return json(route, 200, habit);
    }

    const habitToggleMatch = path.match(/^\/habits\/(\d+)\/toggle$/);
    if (habitToggleMatch && method === 'POST') {
      const id = Number(habitToggleMatch[1]);
      const habit = this.habits.find((h) => h.id === id);
      if (!habit) return json(route, 404, { error: 'Habit not found', code: 404 });
      const todayStr = body?.date || new Date().toISOString().split('T')[0];
      const existing = this.habitLogs.find((l) => l.habit_id === id && l.log_date === todayStr);
      let newCompleted = true;
      if (existing) {
        existing.completed = !existing.completed;
        newCompleted = existing.completed;
      } else {
        this.seedHabitLog({ habit_id: id, log_date: todayStr, completed: true });
      }
      return json(route, 200, { habit_id: id, log_date: todayStr, completed: newCompleted });
    }

    const habitMatch = path.match(/^\/habits\/(\d+)$/);
    if (habitMatch && method === 'PUT') {
      const id = Number(habitMatch[1]);
      const habit = this.habits.find((h) => h.id === id);
      if (!habit) return json(route, 404, { error: 'Habit not found', code: 404 });
      if (body?.name) habit.name = body.name.trim();
      if (body?.description !== undefined) habit.description = body.description;
      if (body?.category) habit.category = body.category;
      if (body?.target_days_per_week) habit.target_days_per_week = body.target_days_per_week;
      habit.updated_at = new Date().toISOString();
      return json(route, 200, habit);
    }

    if (habitMatch && method === 'DELETE') {
      const id = Number(habitMatch[1]);
      const idx = this.habits.findIndex((h) => h.id === id);
      if (idx === -1) return json(route, 404, { error: 'Habit not found', code: 404 });
      this.habits.splice(idx, 1);
      this.habitLogs = this.habitLogs.filter((l) => l.habit_id !== id);
      return json(route, 200, { success: true, id });
    }

    // --- life ---
    if (path === '/life/profile' && method === 'GET') {
      if (!this.lifeProfile) {
        return json(route, 200, { profile: null, stats: null });
      }
      const birth = new Date(this.lifeProfile.birth_date);
      const today = new Date();
      const ageInDays = Math.max(0, Math.floor((today.getTime() - birth.getTime()) / (1000 * 3600 * 24)));
      const totalLifeDays = Math.floor(this.lifeProfile.expected_lifespan_years * 365.25);
      const lifeLived = totalLifeDays > 0 ? Math.round((ageInDays / totalLifeDays) * 10000) / 100 : 0;
      const daysLogged = this.lifeDayLogs.filter((l) => !!l.mood).length;

      const moodCounts: Record<string, number> = {};
      for (const l of this.lifeDayLogs) {
        if (l.mood) {
          moodCounts[l.mood] = (moodCounts[l.mood] || 0) + 1;
        }
      }
      let dominantMood: string | null = null;
      let maxCount = 0;
      for (const [m, count] of Object.entries(moodCounts)) {
        if (count > maxCount) {
          maxCount = count;
          dominantMood = m;
        }
      }

      let totalWorkBlocks = 0;
      let totalSleepBlocks = 0;
      let sleepDayCount = 0;
      for (const l of this.lifeDayLogs) {
        if (l.time_allocation) {
          try {
            const parsed = JSON.parse(l.time_allocation);
            const blocks = Array.isArray(parsed)
              ? parsed
              : Array.isArray(parsed?.blocks)
              ? parsed.blocks
              : [];
            let daySleep = 0;
            for (const b of blocks) {
              if (b === 'deep_work') totalWorkBlocks++;
              else if (b === 'sleep') {
                totalSleepBlocks++;
                daySleep++;
              }
            }
            if (daySleep > 0) sleepDayCount++;
          } catch {
            // Ignore parse errors in mock
          }
        }
      }
      const totalDeepWorkHours = totalWorkBlocks * 0.25;
      const averageSleepHours = sleepDayCount > 0 ? (totalSleepBlocks * 0.25) / sleepDayCount : null;

      return json(route, 200, {
        profile: this.lifeProfile,
        age_days: ageInDays,
        total_expected_days: totalLifeDays,
        days_lived_percent: Math.min(100, lifeLived),
        total_logged_days: daysLogged,
        dominant_mood: dominantMood,
        today_date: new Date().toISOString().split('T')[0],
        stats: {
          birth_date: this.lifeProfile.birth_date,
          expected_lifespan_years: this.lifeProfile.expected_lifespan_years,
          age_in_days: ageInDays,
          total_life_days: totalLifeDays,
          life_lived_percentage: Math.min(100, lifeLived),
          days_logged: daysLogged,
          dominant_mood: dominantMood,
          total_deep_work_hours: totalDeepWorkHours,
          average_sleep_hours: averageSleepHours !== null ? Math.round(averageSleepHours * 10) / 10 : null,
        },
      });
    }

    if (path === '/life/profile' && (method === 'PUT' || method === 'POST')) {
      if (!body?.birth_date) {
        return json(route, 400, { error: 'Birth date is required', code: 400 });
      }
      const lifespan = Number(body.expected_lifespan_years) || 80;
      if (!this.lifeProfile) {
        this.lifeProfile = {
          id: this.nextLifeProfileId++,
          owner_id: OWNER,
          birth_date: body.birth_date,
          expected_lifespan_years: lifespan,
          created_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
        };
      } else {
        this.lifeProfile.birth_date = body.birth_date;
        this.lifeProfile.expected_lifespan_years = lifespan;
        this.lifeProfile.updated_at = new Date().toISOString();
      }
      return json(route, 200, this.lifeProfile);
    }

    const yearMatch = path.match(/^\/life\/year\/(\d+)$/);
    if (yearMatch && method === 'GET') {
      const year = Number(yearMatch[1]);
      const todayStr = new Date().toISOString().split('T')[0];
      const days = [];
      const isLeap = (year % 4 === 0 && year % 100 !== 0) || (year % 400 === 0);
      const daysInYear = isLeap ? 366 : 365;
      const curDate = new Date(Date.UTC(year, 0, 1));
      const logMap = new Map(this.lifeDayLogs.map((l) => [l.log_date, l]));

      for (let i = 0; i < daysInYear; i++) {
        const dateStr = curDate.toISOString().split('T')[0];
        const log = logMap.get(dateStr);
        let day_type = 'past';
        if (dateStr === todayStr) {
          day_type = 'present';
        } else if (dateStr > todayStr) {
          day_type = 'future';
        }
        days.push({
          date: dateStr,
          day_type,
          mood: log?.mood || undefined,
          weather: log?.weather ?? undefined,
          activities: log?.activities ? JSON.parse(log.activities) : undefined,
          note: log?.note ?? undefined,
          time_allocation: log?.time_allocation ?? undefined,
        });
        curDate.setUTCDate(curDate.getUTCDate() + 1);
      }

      return json(route, 200, {
        year,
        birth_date: this.lifeProfile?.birth_date ?? null,
        days,
      });
    }

    if (path === '/life/horizon' && method === 'GET') {
      if (!this.lifeProfile) {
        return json(route, 200, []);
      }
      const birth = new Date(this.lifeProfile.birth_date);
      const todayStr = new Date().toISOString().split('T')[0];
      const logMap = new Map(this.lifeDayLogs.map((l) => [l.log_date, l]));

      const actCountMap = new Map<string, number>();
      const addAct = (dateStr: string | null | undefined) => {
        if (!dateStr) return;
        const d = dateStr.slice(0, 10);
        actCountMap.set(d, (actCountMap.get(d) || 0) + 1);
      };
      for (const t of this.todos) {
        addAct(t.created_at);
        if (t.due_at) addAct(t.due_at);
      }
      for (const tx of this.transactions) {
        addAct(tx.occurred_at);
      }
      for (const hl of this.habitLogs) {
        addAct(hl.log_date);
      }
      for (const r of this.reminders) {
        addAct(r.trigger_at);
      }
      for (const b of this.bookmarks) {
        addAct(b.created_at);
      }

      const totalDays = Math.min(36500, Math.floor(this.lifeProfile.expected_lifespan_years * 365.25));
      const horizon = [];
      const cur = new Date(birth);
      for (let i = 0; i < totalDays; i++) {
        const dateStr = cur.toISOString().split('T')[0];
        const log = logMap.get(dateStr);
        const actCount = actCountMap.get(dateStr) || 0;
        const hasActivity = actCount > 0;
        let status = 'past_empty';
        if (dateStr === todayStr) {
          status = 'present';
        } else if (dateStr > todayStr) {
          status = 'future';
        } else if (log && log.mood) {
          status = 'past_logged';
        } else if (hasActivity) {
          status = 'past_activity';
        }
        horizon.push({
          index: i,
          year: cur.getUTCFullYear(),
          date: dateStr,
          status,
          mood: log?.mood ?? null,
          time_allocation: log?.time_allocation ?? null,
          has_activity: hasActivity,
          activity_count: actCount,
        });
        cur.setUTCDate(cur.getUTCDate() + 1);
      }
      return json(route, 200, horizon);
    }

    if (path === '/life/days' && method === 'POST') {
      const logDate = body?.log_date || body?.date;
      if (!logDate || !body?.mood) {
        return json(route, 400, { error: 'Date and mood are required', code: 400 });
      }
      const existingIdx = this.lifeDayLogs.findIndex((l) => l.log_date === logDate);
      if (existingIdx >= 0) {
        const ex = this.lifeDayLogs[existingIdx];
        if (ex.mood && ex.mood.length > 0) {
          return json(route, 400, {
            error: 'This day has already been logged and cannot be edited. It can only be cleared.',
            code: 400,
          });
        }
      }
      const activitiesStr = body.activities ? JSON.stringify(body.activities) : null;
      const timeAllocationStr = body.time_allocation ? JSON.stringify(body.time_allocation) : null;

      if (existingIdx >= 0) {
        const ex = this.lifeDayLogs[existingIdx];
        ex.mood = body.mood;
        ex.weather = body.weather ?? ex.weather;
        ex.activities = activitiesStr ?? ex.activities;
        ex.note = body.note ?? ex.note;
        if (timeAllocationStr) {
          ex.time_allocation = timeAllocationStr;
        }
        ex.updated_at = new Date().toISOString();
        return json(route, 200, ex);
      }

      const saved: FakeLifeDayLog = {
        id: this.nextLifeDayLogId++,
        owner_id: OWNER,
        log_date: logDate,
        mood: body.mood,
        weather: body.weather ?? null,
        activities: activitiesStr,
        note: body.note ?? null,
        time_allocation: timeAllocationStr,
        created_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
      };
      this.lifeDayLogs.push(saved);
      return json(route, 200, saved);
    }

    const dayTimeMatch = path.match(/^\/life\/days\/(\d{4}-\d{2}-\d{2})\/time$/);
    if (dayTimeMatch && method === 'PUT') {
      const date = dayTimeMatch[1];
      const timeAllocationStr = body?.time_allocation ? JSON.stringify(body.time_allocation) : null;
      let log = this.lifeDayLogs.find((l) => l.log_date === date);
      if (log) {
        log.time_allocation = timeAllocationStr;
        log.updated_at = new Date().toISOString();
      } else {
        log = {
          id: this.nextLifeDayLogId++,
          owner_id: OWNER,
          log_date: date,
          mood: '',
          weather: null,
          activities: null,
          note: null,
          time_allocation: timeAllocationStr,
          created_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
        };
        this.lifeDayLogs.push(log);
      }
      return json(route, 200, log);
    }

    const dayActivityMatch = path.match(/^\/life\/days\/(\d{4}-\d{2}-\d{2})\/activity$/);
    if (dayActivityMatch && method === 'GET') {
      const date = dayActivityMatch[1];
      const events: any[] = [];
      const stats = {
        total_events: 0,
        todos_created: 0,
        todos_completed: 0,
        todos_due: 0,
        expenses: 0,
        income: 0,
        portfolio_trades: 0,
        habits_done: 0,
        reminders_fired: 0,
        bookmarks_saved: 0,
        bills_paid: 0,
        footprints: 0,
      };

      for (const todo of this.todos) {
        if (todo.created_at && todo.created_at.startsWith(date)) {
          events.push({
            time: todo.created_at.split('T')[1]?.slice(0, 8) || null,
            module: 'todo',
            action: 'created',
            emoji: '📝',
            summary: todo.text,
            detail: `priority: ${todo.priority}`,
          });
          stats.todos_created++;
        }
        if (todo.completed && todo.created_at && todo.created_at.startsWith(date)) {
          events.push({
            time: todo.created_at.split('T')[1]?.slice(0, 8) || null,
            module: 'todo',
            action: 'completed',
            emoji: '✔️',
            summary: todo.text,
            detail: null,
          });
          stats.todos_completed++;
        }
      }

      for (const tx of this.transactions) {
        if (tx.occurred_at && tx.occurred_at.startsWith(date)) {
          const isExpense = tx.direction === 'expense';
          events.push({
            time: tx.occurred_at.split('T')[1]?.slice(0, 8) || null,
            module: 'finance',
            action: tx.direction,
            emoji: isExpense ? '💸' : '💰',
            summary: `${isExpense ? '-' : '+'}${tx.currency}${(tx.amount_minor / 100).toFixed(2)}`,
            detail: tx.note || null,
          });
          if (isExpense) stats.expenses++;
          else stats.income++;
        }
      }

      for (const hl of this.habitLogs) {
        if (hl.log_date === date) {
          const habit = this.habits.find((h) => h.id === hl.habit_id);
          events.push({
            time: null,
            module: 'habit',
            action: 'checked',
            emoji: '✅',
            summary: habit?.name || 'Habit',
            detail: hl.note || null,
          });
          stats.habits_done++;
        }
      }

      stats.total_events = events.length;
      return json(route, 200, { date, events, stats });
    }

    const dayMatch = path.match(/^\/life\/days\/(\d{4}-\d{2}-\d{2})$/);
    if (dayMatch && method === 'GET') {
      const date = dayMatch[1];
      const log = this.lifeDayLogs.find((l) => l.log_date === date) || null;
      return json(route, 200, log);
    }
    if (dayMatch && method === 'DELETE') {
      const date = dayMatch[1];
      const idx = this.lifeDayLogs.findIndex((l) => l.log_date === date);
      if (idx >= 0) {
        this.lifeDayLogs.splice(idx, 1);
      }
      return json(route, 200, { date, deleted: true });
    }

    // --- recurring bills ---
    if (path === '/recurring-bills' && method === 'GET') {
      return json(route, 200, this.bills);
    }
    if (path === '/recurring-bills' && method === 'POST') {
      const currency = String(body?.currency ?? 'USD').toUpperCase();
      const bill = this.seedRecurringBill({
        name: String(body?.name ?? ''),
        category: String(body?.category ?? 'subscription'),
        amount_minor: Number(body?.amount_minor ?? 0),
        currency,
        currency_exponent: Number(body?.currency_exponent ?? exponentFor(currency)),
        frequency: String(body?.frequency ?? 'monthly'),
        due_day: body?.due_day ?? null,
        next_due_at: String(body?.next_due_at ?? new Date().toISOString()),
        notes: body?.notes ?? null,
      });
      return json(route, 200, bill);
    }
    const payMatch = path.match(/^\/recurring-bills\/(\d+)\/pay$/);
    if (payMatch && method === 'POST') {
      const id = Number(payMatch[1]);
      const bill = this.bills.find((b) => b.id === id);
      if (!bill) return json(route, 404, { error: 'Bill not found', code: 404 });
      bill.paid_installments += 1;
      bill.updated_at = new Date().toISOString();
      this.seedTransaction({
        direction: 'expense',
        amount_minor: bill.amount_minor,
        currency: bill.currency,
        currency_exponent: bill.currency_exponent,
        category_name: bill.category,
        note: `Recurring bill: ${bill.name}`,
        source_kind: 'recurring_bill',
        source_id: bill.id,
      });
      return json(route, 200, bill);
    }
    const billIdMatch = path.match(/^\/recurring-bills\/(\d+)$/);
    if (billIdMatch && (method === 'PATCH' || method === 'PUT')) {
      const id = Number(billIdMatch[1]);
      const bill = this.bills.find((b) => b.id === id);
      if (!bill) return json(route, 404, { error: 'Bill not found', code: 404 });
      if (body?.name !== undefined) bill.name = String(body.name);
      if (body?.category !== undefined) bill.category = String(body.category);
      if (body?.currency !== undefined) {
        bill.currency = String(body.currency).toUpperCase();
        bill.currency_exponent = exponentFor(bill.currency);
      }
      if (body?.amount_minor !== undefined) bill.amount_minor = Number(body.amount_minor);
      if (body?.frequency !== undefined) bill.frequency = String(body.frequency);
      if (body?.due_day !== undefined) bill.due_day = body.due_day;
      if (body?.next_due_at !== undefined) bill.next_due_at = String(body.next_due_at);
      if (body?.notes !== undefined) bill.notes = body.notes;
      if (body?.status !== undefined) bill.status = String(body.status);
      bill.updated_at = new Date().toISOString();
      return json(route, 200, bill);
    }
    if (billIdMatch && method === 'DELETE') {
      const id = Number(billIdMatch[1]);
      const idx = this.bills.findIndex((b) => b.id === id);
      if (idx >= 0) {
        this.bills.splice(idx, 1);
      }
      return json(route, 200, { deleted: true });
    }

    // --- bookmarks ---
    if (path === '/bookmarks' && method === 'GET') {
      const url = new URL(route.request().url());
      const archivedParam = url.searchParams.get('archived');
      const favoriteParam = url.searchParams.get('favorite');
      const tagParam = url.searchParams.get('tag');
      const searchParam = url.searchParams.get('search')?.toLowerCase();
      const domainParam = url.searchParams.get('domain');

      let list = [...this.bookmarks];
      if (archivedParam !== null) {
        const isArch = archivedParam === 'true';
        list = list.filter((b) => b.is_archived === isArch);
      }
      if (favoriteParam !== null) {
        const isFav = favoriteParam === 'true';
        list = list.filter((b) => b.is_favorite === isFav);
      }
      if (tagParam) {
        list = list.filter((b) =>
          b.tags
            .split(',')
            .map((t) => t.trim().toLowerCase())
            .includes(tagParam.toLowerCase())
        );
      }
      if (domainParam) {
        list = list.filter((b) => b.domain.toLowerCase() === domainParam.toLowerCase());
      }
      if (searchParam) {
        list = list.filter(
          (b) =>
            b.title.toLowerCase().includes(searchParam) ||
            (b.description && b.description.toLowerCase().includes(searchParam)) ||
            b.url.toLowerCase().includes(searchParam) ||
            b.tags.toLowerCase().includes(searchParam)
        );
      }
      return json(route, 200, list);
    }

    if (path === '/bookmarks/stats' && method === 'GET') {
      const total_count = this.bookmarks.length;
      const unread_count = this.bookmarks.filter((b) => !b.is_archived).length;
      const favorite_count = this.bookmarks.filter((b) => b.is_favorite).length;
      const archived_count = this.bookmarks.filter((b) => b.is_archived).length;

      const tagSet = new Set<string>();
      const domainMap = new Map<string, number>();
      for (const b of this.bookmarks) {
        for (const t of b.tags.split(',')) {
          const trimmed = t.trim();
          if (trimmed) tagSet.add(trimmed);
        }
        domainMap.set(b.domain, (domainMap.get(b.domain) || 0) + 1);
      }
      const top_domains = Array.from(domainMap.entries())
        .sort((a, b) => b[1] - a[1])
        .slice(0, 10)
        .map(([domain, count]) => ({ domain, count }));

      return json(route, 200, {
        total_count,
        unread_count,
        favorite_count,
        archived_count,
        tags: Array.from(tagSet),
        top_domains,
      });
    }

    if (path === '/bookmarks' && method === 'POST') {
      if (!body?.url?.trim()) {
        return json(route, 400, { error: 'url is required', code: 400 });
      }
      const rawUrl = body.url.trim();
      let domain = 'example.com';
      try {
        domain = new URL(rawUrl).hostname;
      } catch (_) {}
      const tagsStr = Array.isArray(body.tags) ? body.tags.join(',') : '';
      const bookmark = this.seedBookmark({
        url: rawUrl,
        title: body.title || 'Saved Page',
        domain,
        tags: tagsStr,
        reader_content: `This is the readable extracted article text for ${rawUrl}.\n\nIt features distraction-free reading with paragraphs cleanly displayed.`,
      });
      return json(route, 200, bookmark);
    }

    const bookmarkToggleFav = path.match(/^\/bookmarks\/(\d+)\/toggle-favorite$/);
    if (bookmarkToggleFav && method === 'POST') {
      const id = Number(bookmarkToggleFav[1]);
      const bm = this.bookmarks.find((b) => b.id === id);
      if (!bm) return json(route, 404, { error: 'Bookmark not found', code: 404 });
      bm.is_favorite = !bm.is_favorite;
      bm.updated_at = new Date().toISOString();
      return json(route, 200, bm);
    }

    const bookmarkToggleArch = path.match(/^\/bookmarks\/(\d+)\/toggle-archive$/);
    if (bookmarkToggleArch && method === 'POST') {
      const id = Number(bookmarkToggleArch[1]);
      const bm = this.bookmarks.find((b) => b.id === id);
      if (!bm) return json(route, 404, { error: 'Bookmark not found', code: 404 });
      bm.is_archived = !bm.is_archived;
      bm.updated_at = new Date().toISOString();
      return json(route, 200, bm);
    }

    const bookmarkIdMatch = path.match(/^\/bookmarks\/(\d+)$/);
    if (bookmarkIdMatch && method === 'GET') {
      const id = Number(bookmarkIdMatch[1]);
      const bm = this.bookmarks.find((b) => b.id === id);
      if (!bm) return json(route, 404, { error: 'Bookmark not found', code: 404 });
      return json(route, 200, bm);
    }

    if (bookmarkIdMatch && method === 'PATCH') {
      const id = Number(bookmarkIdMatch[1]);
      const bm = this.bookmarks.find((b) => b.id === id);
      if (!bm) return json(route, 404, { error: 'Bookmark not found', code: 404 });
      if (body?.title !== undefined) bm.title = body.title;
      if (body?.description !== undefined) bm.description = body.description;
      if (body?.is_favorite !== undefined) bm.is_favorite = body.is_favorite;
      if (body?.is_archived !== undefined) bm.is_archived = body.is_archived;
      if (Array.isArray(body?.tags)) bm.tags = body.tags.join(',');
      bm.updated_at = new Date().toISOString();
      return json(route, 200, bm);
    }

    if (bookmarkIdMatch && method === 'DELETE') {
      const id = Number(bookmarkIdMatch[1]);
      const idx = this.bookmarks.findIndex((b) => b.id === id);
      if (idx === -1) return json(route, 404, { error: 'Bookmark not found', code: 404 });
      this.bookmarks.splice(idx, 1);
      return json(route, 200, { success: true, id });
    }

    // --- notes & knowledge ---
    if (path === '/notes/tags' && method === 'GET') {
      const allTags = Array.from(new Set(this.notes.flatMap((n) => n.tags))).sort();
      return json(route, 200, allTags);
    }

    if (path === '/notes/export' && method === 'GET') {
      return route.fulfill({
        status: 200,
        headers: {
          'content-type': 'application/zip',
          'content-disposition': 'attachment; filename="notes-2026-10-09.zip"',
        },
        // An empty zip: just its end-of-central-directory record.
        body: Buffer.from('504b0506000000000000000000000000000000000000', 'hex'),
      });
    }

    if (path === '/notes/import' && method === 'POST') {
      // The body is multipart, so only the uploaded file's name is recorded.
      const raw = route.request().postDataBuffer()?.toString('latin1') ?? '';
      const name = raw.match(/filename="([^"]+)"/)?.[1];
      if (name) this.importedFileNames.push(name);
      return json(route, 200, this.importReport);
    }

    if (path === '/notes' && method === 'GET') {
      const u = url || new URL(route.request().url());
      const archivedParam = u.searchParams.get('archived');
      const tagParam = u.searchParams.get('tag');
      const searchParam = u.searchParams.get('search')?.toLowerCase();

      let list = [...this.notes];
      if (archivedParam !== null) {
        const isArch = archivedParam === 'true';
        list = list.filter((n) => n.is_archived === isArch);
      }
      if (tagParam) {
        list = list.filter((n) => n.tags.includes(tagParam));
      }
      if (searchParam) {
        list = list.filter(
          (n) =>
            n.title.toLowerCase().includes(searchParam) ||
            n.content.toLowerCase().includes(searchParam)
        );
      }
      return json(route, 200, list.map(noteSummary));
    }

    if (path === '/notes' && method === 'POST') {
      if (!body?.title?.trim() || !body?.content?.trim()) {
        return json(route, 400, { error: 'title and content are required', code: 400 });
      }
      const note = this.seedNote({
        title: body.title.trim(),
        content: body.content.trim(),
        note_type: body.note_type || 'note',
        tags: Array.isArray(body.tags) ? body.tags : [],
      });
      return json(route, 201, noteDocument(note));
    }

    const noteIdMatch = path.match(/^\/notes\/(\d+)$/);
    if (noteIdMatch && method === 'GET') {
      const id = Number(noteIdMatch[1]);
      const note = this.notes.find((n) => n.id === id);
      if (!note) return json(route, 404, { error: 'Note not found', code: 404 });
      return json(route, 200, noteDocument(note));
    }

    if (noteIdMatch && method === 'PUT') {
      const id = Number(noteIdMatch[1]);
      const note = this.notes.find((n) => n.id === id);
      if (!note) return json(route, 404, { error: 'Note not found', code: 404 });
      if (body?.expected_content_hash && body.expected_content_hash !== noteHash(note.content)) {
        return json(route, 409, { error: 'note changed since it was loaded', code: 409 });
      }
      if (body?.title !== undefined) note.title = body.title;
      if (body?.content !== undefined) note.content = body.content;
      if (body?.note_type !== undefined) note.note_type = body.note_type;
      if (body?.tags !== undefined) note.tags = body.tags;
      if (body?.is_archived !== undefined) note.is_archived = body.is_archived;
      note.updated_at = new Date().toISOString();
      return json(route, 200, noteDocument(note));
    }

    if (noteIdMatch && method === 'DELETE') {
      const id = Number(noteIdMatch[1]);
      const idx = this.notes.findIndex((n) => n.id === id);
      if (idx === -1) return json(route, 404, { error: 'Note not found', code: 404 });
      this.notes.splice(idx, 1);
      return route.fulfill({ status: 204 });
    }

    if (path === '/knowledge/search' && method === 'POST') {
      const q = (body?.query || '').toLowerCase().trim();
      const results = this.notes
        .filter(
          (n) =>
            !n.is_archived &&
            (n.title.toLowerCase().includes(q) || n.content.toLowerCase().includes(q))
        )
        .map((n) => ({
          item_type: 'note',
          item_id: n.id,
          title: n.title,
          content_snippet: n.content.slice(0, 150),
          similarity_score: 0.92,
          source_ref: n.source_ref,
        }));
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        headers: { 'x-search-mode': this.searchMode },
        body: JSON.stringify(results),
      });
    }

    // --- ocr scan ---
    if (path === '/ocr/scan' && method === 'POST') {
      return json(route, 200, {
        raw_text: 'STARBUCKS COFFEE\n123 MAIN ST\nTOTAL $9.99\nDATE 2026-09-15',
        blocks: [
          { text: 'STARBUCKS COFFEE', score: 0.98 },
          { text: '123 MAIN ST', score: 0.95 },
          { text: 'TOTAL $9.99', score: 0.99 },
          { text: 'DATE 2026-09-15', score: 0.97 },
        ],
        structured: {
          merchant: 'STARBUCKS COFFEE',
          amount: '9.99',
          currency: 'USD',
          date: '2026-09-15',
          category_suggestion: 'Dining',
        },
      });
    }

    // --- voice transcribe ---
    if (path === '/voice/transcribe' && method === 'POST') {
      return json(route, 200, {
        text: 'Spent $15 on lunch at Subway',
      });
    }

    // --- morning briefing ---
    if (path === '/briefing/trigger' && method === 'POST') {
      return json(route, 200, {
        owner_id: 'acct_00000000000000000000000000000001',
        html_content: '<h3>☀️ 早安简报</h3>',
        plain_text: '☀️ 早安简报',
        summary: {
          pending_tasks_count: this.todos.filter((t) => !t.completed).length,
          habits_completed_count: 0,
          habits_total_count: this.habits.length,
          upcoming_bills_count: this.recurringBills.length,
          holdings_count: this.holdings.length,
        },
        tickers: [],
      });
    }

    // --- agent runs & approvals ---
    if (path === '/agent/runs' && method === 'GET') {
      return json(route, 200, this.agentRuns);
    }

    if (path === '/agent/runs' && method === 'POST') {
      const newRun: FakeAgentRun = {
        id: `run-${Date.now()}`,
        scheduled_task_id: null,
        account_id: 'acct_00000000000000000000000000000001',
        task_name: 'ad_hoc_goal',
        goal: body?.goal || 'New agent goal',
        origin_conversation: body?.origin_conversation || 'web',
        status: 'running',
        max_steps: body?.max_steps || 10,
        current_step: 1,
        total_tokens: 150,
        error: null,
        started_at: new Date().toISOString(),
        completed_at: null,
        updated_at: new Date().toISOString(),
      };
      this.agentRuns.unshift(newRun);
      this.agentSteps[newRun.id] = [
        {
          id: `step-${newRun.id}-1`,
          run_id: newRun.id,
          step_number: 1,
          thought: 'Analyzing initial request...',
          tool_name: null,
          tool_input: null,
          tool_output: 'Initialized goal execution.',
          status: 'completed',
          tokens_used: 150,
          created_at: new Date().toISOString(),
        },
      ];
      return json(route, 200, newRun);
    }

    if (path.startsWith('/agent/runs/') && path.endsWith('/cancel') && method === 'POST') {
      const id = path.replace('/agent/runs/', '').replace('/cancel', '');
      const run = this.agentRuns.find((r) => r.id === id);
      if (!run) return json(route, 404, { error: 'Run not found', code: 404 });
      run.status = 'cancelled';
      run.updated_at = new Date().toISOString();
      return json(route, 200, run);
    }

    if (path.startsWith('/agent/runs/') && method === 'GET') {
      const id = path.replace('/agent/runs/', '');
      const run = this.agentRuns.find((r) => r.id === id);
      if (!run) return json(route, 404, { error: 'Run not found', code: 404 });
      return json(route, 200, { run, steps: this.agentSteps[id] || [] });
    }

    if (path === '/agent/approvals' && method === 'GET') {
      return json(route, 200, this.agentApprovals);
    }

    if (path.startsWith('/agent/approvals/') && path.endsWith('/approve') && method === 'POST') {
      const id = path.replace('/agent/approvals/', '').replace('/approve', '');
      const approval = this.agentApprovals.find((a) => a.id === id);
      if (!approval) return json(route, 404, { error: 'Approval not found', code: 404 });
      approval.status = 'approved';
      approval.resolved_at = new Date().toISOString();
      const run = this.agentRuns.find((r) => r.id === approval.run_id);
      if (run) {
        run.status = 'running';
        run.updated_at = new Date().toISOString();
      }
      return json(route, 200, approval);
    }

    if (path.startsWith('/agent/approvals/') && path.endsWith('/reject') && method === 'POST') {
      const id = path.replace('/agent/approvals/', '').replace('/reject', '');
      const approval = this.agentApprovals.find((a) => a.id === id);
      if (!approval) return json(route, 404, { error: 'Approval not found', code: 404 });
      approval.status = 'rejected';
      approval.resolved_at = new Date().toISOString();
      return json(route, 200, approval);
    }

    if (path === '/agent/tasks' && method === 'GET') {
      return json(route, 200, this.agentTasks);
    }

    if (path.startsWith('/agent/tasks/') && method === 'PUT') {
      const id = decodeURIComponent(path.replace('/agent/tasks/', ''));
      const task = this.agentTasks.find((t) => t.task_id === id);
      if (!task) return json(route, 404, { error: 'Task not found', code: 404 });
      if (body?.cron_expression) task.cron_expression = body.cron_expression;
      if (body?.config !== undefined) task.config = body.config;
      task.updated_at = new Date().toISOString();
      return json(route, 200, task);
    }

    return json(route, 404, { error: `Unhandled route ${method} ${path}`, code: 404 });
  }

  private assertLead(route: Route, minutes: unknown) {
    const value = Number(minutes);
    if (!Number.isFinite(value) || value <= 0) {
      return json(route, 400, {
        error: 'remind_before_minutes must be greater than zero; send null to clear it',
        code: 400,
      });
    }
    if (value > MAX_LEAD_MINUTES) {
      return json(route, 400, {
        error: `remind_before_minutes must be at most ${MAX_LEAD_MINUTES} (30 days)`,
        code: 400,
      });
    }
    return null;
  }
}

function json(route: Route, status: number, payload: unknown) {
  return route.fulfill({
    status,
    contentType: 'application/json',
    body: JSON.stringify(payload),
  });
}

/** Mirrors `modules::finance::money::exponent_for`. */
function exponentFor(currency: string): number {
  const zeroDecimal = ['JPY', 'KRW', 'VND', 'CLP', 'ISK'];
  const threeDecimal = ['BHD', 'KWD', 'OMR', 'TND', 'JOD'];
  return zeroDecimal.includes(currency) ? 0 : threeDecimal.includes(currency) ? 3 : 2;
}

/**
 * Mirrors `modules::finance::money::parse_amount` — currency-agnostic, just an
 * exponent, so `portfolio` reuses it for both price and quantity server-side;
 * the fake mirrors that reuse rather than hand-rolling a second parser.
 */
function parseDecimal(raw: string, exponent: number): { minor: number } | { error: string } {
  const cleaned = raw.trim().replace(/[$€£¥₹,\s]/g, '');
  if (cleaned.startsWith('-')) return { error: 'Amount must be greater than zero' };
  if (!/^\d*\.?\d*$/.test(cleaned) || cleaned === '' || cleaned === '.') {
    return { error: 'Amount must be a number such as 15.50' };
  }
  const [whole, fraction = ''] = cleaned.split('.');
  if (fraction.length > exponent) {
    return { error: `That currency has ${exponent} decimal place(s)` };
  }
  const minor = Number(`${whole || '0'}${fraction.padEnd(exponent, '0')}`);
  if (!minor) return { error: 'Amount must be greater than zero' };
  return { minor };
}

/** The inverse of `parseDecimal` — mirrors `modules::finance::money::format_amount`,
 *  used for error messages that state an amount back to the user. */
function formatDecimal(minor: number, exponent: number): string {
  if (exponent === 0) return String(minor);
  const text = Math.abs(minor).toString().padStart(exponent + 1, '0');
  const whole = text.slice(0, text.length - exponent);
  const fraction = text.slice(text.length - exponent);
  return `${minor < 0 ? '-' : ''}${whole}.${fraction}`;
}

/**
 * Mirrors `modules::finance::money` — minor units by string manipulation, never
 * through a float, and per-currency decimal places.
 */
function parseAmount(
  raw: string,
  currencyRaw: string
): { minor: number; currency: string; exponent: number } | { error: string } {
  const currency = currencyRaw.trim().toUpperCase();
  if (!/^[A-Z]{3}$/.test(currency)) {
    return { error: 'Currency must be a 3-letter code such as EUR or JPY' };
  }

  const zeroDecimal = ['JPY', 'KRW', 'VND', 'CLP', 'ISK'];
  const threeDecimal = ['BHD', 'KWD', 'OMR', 'TND', 'JOD'];
  const exponent = zeroDecimal.includes(currency) ? 0 : threeDecimal.includes(currency) ? 3 : 2;

  const cleaned = raw.trim().replace(/[$€£¥₹,\s]/g, '');
  if (cleaned.startsWith('-')) return { error: 'Amount must be greater than zero' };
  if (!/^\d*\.?\d*$/.test(cleaned) || cleaned === '' || cleaned === '.') {
    return { error: 'Amount must be a number such as 15.50' };
  }

  const [whole, fraction = ''] = cleaned.split('.');
  if (fraction.length > exponent) {
    return { error: `That currency has ${exponent} decimal place(s)` };
  }

  const minor = Number(`${whole || '0'}${fraction.padEnd(exponent, '0')}`);
  if (!minor) return { error: 'Amount must be greater than zero' };

  return { minor, currency, exponent };
}

/**
 * How long the browser waits for an authenticator, in the canned challenges.
 *
 * Production sends 300s. Here a ceremony that cannot complete would otherwise
 * hang a worker for five minutes and look like a stuck suite rather than a
 * failing test.
 */
const CEREMONY_TIMEOUT_MS = 10_000;

/**
 * A base64url string of `length` bytes, which is how WebAuthn carries binary
 * over JSON. The bytes themselves are arbitrary — nothing verifies them here.
 */
function randomB64Url(length: number): string {
  const bytes = Array.from({ length }, (_, i) => (i * 37) % 251);
  return btoa(String.fromCharCode(...bytes)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

/** Mirrors what `webauthn-rs` sends for `start_passkey_registration`. */
function registrationOptions(displayName: string) {
  return {
    rp: { name: 'Matrix Bot', id: 'localhost' },
    user: { id: randomB64Url(16), name: displayName, displayName },
    challenge: randomB64Url(32),
    pubKeyCredParams: [
      { type: 'public-key', alg: -7 },
      { type: 'public-key', alg: -257 },
    ],
    timeout: CEREMONY_TIMEOUT_MS,
    excludeCredentials: [],
    authenticatorSelection: { residentKey: 'required', userVerification: 'required' },
    attestation: 'none',
  };
}

/** Mirrors `start_discoverable_authentication`: no `allowCredentials`. */
function authenticationOptions() {
  return {
    challenge: randomB64Url(32),
    timeout: CEREMONY_TIMEOUT_MS,
    rpId: 'localhost',
    allowCredentials: [],
    userVerification: 'required',
  };
}
