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

const OWNER = '@web-tester:api';
const DISPLAY_NAME = 'tester';
/** Mirrors `due_reminder::MAX_LEAD_MINUTES`. */
const MAX_LEAD_MINUTES = 60 * 24 * 30;

export class FakeApi {
  todos: FakeTodo[] = [];
  reminders: FakeReminder[] = [];
  rooms: FakeRoom[] = [
    { room_id: '!general:mbm.one', name: 'General' },
    { room_id: '!ops:mbm.one', name: 'Ops' },
  ];

  /** Requests seen, so tests can assert on what the UI actually sent. */
  requests: { method: string; url: string; body: unknown }[] = [];

  /** Set to make the next matching call fail, for error-path tests. */
  failNext: { path: RegExp; status: number; error: string } | null = null;

  transactions: FakeTransaction[] = [];
  categories: FakeCategory[] = [];

  private nextTodoId = 1;
  private nextReminderId = 1;
  private nextTxId = 1;
  private nextCategoryId = 1;

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
    if (path === '/auth/login' && method === 'POST') {
      if (body?.password === 'wrong') {
        return json(route, 401, { error: 'Invalid username or password', code: 401 });
      }
      return json(route, 200, { token: 'fake-jwt-token', username: body?.username ?? DISPLAY_NAME });
    }
    if (path === '/auth/register' && method === 'POST') {
      if (String(body?.username ?? '').includes('@')) {
        return json(route, 400, { error: 'Username cannot contain @ or :', code: 400 });
      }
      return json(route, 200, { token: 'fake-jwt-token', username: body?.username ?? DISPLAY_NAME });
    }
    if (path === '/auth/me') {
      return json(route, 200, { username: DISPLAY_NAME, subject: OWNER, kind: 'Local' });
    }

    // --- infrastructure ---
    if (path === '/health') {
      return json(route, 200, { status: 'ok', database: 'connected', version: '0.1.0' });
    }
    if (path === '/events/ticket' && method === 'POST') {
      return json(route, 200, { ticket: 'fake-ticket' });
    }
    if (path === '/events') {
      // Terminate immediately rather than holding a stream open: the app
      // reconnects with backoff, which keeps tests deterministic.
      return route.fulfill({ status: 200, contentType: 'text/event-stream', body: '' });
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
