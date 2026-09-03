import type { Page, Route } from '@playwright/test';

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
    api_base_url: 'http://localhost:11434/v1',
    api_key: null,
    model_name: 'llama3.2',
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

  /** A merge waiting on confirmation, as `!link` would have recorded. */
  pendingMerge: { from_account: string; preview: Record<string, number> } | null = null;

  transactions: FakeTransaction[] = [];
  categories: FakeCategory[] = [];
  holdings: FakeHolding[] = [];
  cash: FakeCashBalance[] = [];
  alerts: any[] = [];
  /** Mirrors the server default (`modules::settings::UserSettings::default`). */
  settings: { timezone: string; currency: string } = { timezone: 'UTC', currency: 'USD' };

  private nextTodoId = 1;
  private nextReminderId = 1;
  private nextTxId = 1;
  private nextCategoryId = 1;
  private nextHoldingId = 1;
  private nextCashId = 1;

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

  async install(page: Page) {
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

      return this.handle(route, method, path, body as Record<string, any> | null);
    });
  }

  private async handle(
    route: Route,
    method: string,
    path: string,
    body: Record<string, any> | null
  ) {
    // --- auth ---
    // The passkey ceremonies are answered with canned challenges. A real
    // authenticator (Playwright's CDP virtual one) still runs, so what these
    // specs exercise is the browser half: the base64url conversions, the
    // options mapping, and how the UI reacts. Verifying an assertion is the
    // server's job and is covered by the Rust tests.
    if (path === '/auth/passkey/register/start' && method === 'POST') {
      return json(route, 200, {
        ceremony_id: 'ceremony_register',
        options: { publicKey: registrationOptions(String(body?.display_name ?? DISPLAY_NAME)) },
      });
    }
    if (path === '/auth/passkey/register/finish' && method === 'POST') {
      this.signedIn = true;
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
      });
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
    if (path === '/account/matrix' && method === 'GET') {
      return json(route, 200, this.linkedMxids);
    }
    if (path === '/account/matrix/link' && method === 'POST') {
      return json(route, 200, { code: 'A3F9-K2QX', expires_in_seconds: 300 });
    }
    if (path === '/account/matrix/unlink' && method === 'POST') {
      this.linkedMxids = this.linkedMxids.filter((m) => m !== body?.mxid);
      return route.fulfill({ status: 204, body: '' });
    }
    if (path === '/account/matrix/pending-merge' && method === 'GET') {
      return json(route, 200, this.pendingMerge);
    }
    if (path === '/account/matrix/confirm-merge' && method === 'POST') {
      const preview = this.pendingMerge?.preview ?? {};
      this.pendingMerge = null;
      return json(route, 200, preview);
    }
    if (path === '/account/sessions' && method === 'GET') {
      return json(route, 200, this.sessions);
    }
    if (path === '/account/sessions/revoke-others' && method === 'POST') {
      const revoked = this.sessions.filter((x) => !x.current).length;
      this.sessions = this.sessions.filter((x) => x.current);
      return json(route, 200, revoked);
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
    if (path === '/health') {
      return json(route, 200, { status: 'ok', database: 'connected', version: '0.1.0' });
    }
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
      return json(route, 200, {
        items: this.todos,
        page: 1,
        limit: 100,
        total: this.todos.length,
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
      return json(route, 200, {
        items: this.transactions,
        page: 1,
        limit: 200,
        total: this.transactions.length,
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

    // --- reminders ---
    if (path === '/reminders' && method === 'GET') {
      const pending = this.reminders.filter((r) => r.status === 'pending');
      return json(route, 200, {
        items: pending,
        page: 1,
        limit: 100,
        total: pending.length,
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
    const reminderMatch = path.match(/^\/reminders\/(\d+)$/);
    if (reminderMatch && method === 'DELETE') {
      const id = Number(reminderMatch[1]);
      const reminder = this.reminders.find((r) => r.id === id);
      if (!reminder) return json(route, 404, { error: 'Reminder not found', code: 404 });
      reminder.status = 'cancelled';
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
      const totalCash = this.cash.reduce((acc, c) => acc + c.balance_minor, 0);
      const totalHoldings = this.holdings.reduce((acc, h) => {
        const p = h.current_price_minor ?? h.avg_cost_minor;
        return acc + Math.trunc((h.quantity_minor * p) / 10 ** QUANTITY_EXPONENT);
      }, 0);
      const total = totalCash + totalHoldings;
      const today = new Date().toISOString().split('T')[0];
      return json(route, 200, {
        currency: 'USD',
        currency_exponent: 2,
        timeframe: '1m',
        points: [
          { date: '2026-08-01', holdings_value_minor: Math.trunc(totalHoldings * 0.9), cash_minor: totalCash, total_net_worth_minor: Math.trunc(total * 0.95) },
          { date: today, holdings_value_minor: totalHoldings, cash_minor: totalCash, total_net_worth_minor: total },
        ],
        starting_value_minor: Math.trunc(total * 0.95),
        current_value_minor: total,
        period_change_minor: Math.trunc(total * 0.05),
        period_change_pct: 5.26,
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

    if (path === '/ai/chat' && method === 'POST') {
      let conv = this.aiConversations.find((c) => c.id === body?.conversation_id);
      if (!conv) {
        conv = {
          id: `conv_${this.nextAiConvId++}`,
          owner_id: OWNER,
          room_id: String(body?.room_id || 'web_chat'),
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

      const isTaskQuestion = body?.prompt?.toLowerCase().includes('task');
      const toolCalls = isTaskQuestion ? ['get_my_tasks'] : [];

      if (isTaskQuestion) {
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
      }

      const botReply = isTaskQuestion
        ? `You have ${this.todos.length} task(s) currently recorded in your list.`
        : `I received your prompt: "${body?.prompt}". How can I help further?`;

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
        model_used: this.aiConfig.model_name || 'llama3.2',
        tool_calls_executed: toolCalls,
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
