import { test, expect, gotoApp, todoRow, TAB } from './fixtures';

/**
 * The live stream.
 *
 * Every mutation in this app is supposed to announce itself, so a change made
 * from Matrix — or from another browser — reaches an open page without a
 * reload. Nothing exercised that until now: the fake answered `/events` with an
 * empty body, so the app sat in "Reconnecting…" for the whole suite and every
 * `handleSseEvent` in every module was dead code as far as the tests knew.
 *
 * Two things are load-bearing and easy to get wrong, so both are asserted here
 * rather than assumed:
 *
 * 1. Messages are **unnamed**. The body carries `type` and `payload` and the
 *    frontend discriminates on `type` from `EventSource.onmessage`. Naming the
 *    event would mean `onmessage` never fires at all.
 * 2. `type` is the Rust variant name **verbatim** — `TodoCreated`, not
 *    `todo_created`. A mismatch is silent: the frame arrives, no handler
 *    matches, and the UI simply never updates.
 */

test('a task created elsewhere appears without a reload', async ({ app, api }) => {
  await gotoApp(app, TAB.todo);
  await expect(todoRow(app, 'From Matrix')).toHaveCount(0);

  // As the bot would broadcast it after `!todo add`.
  api.seedTodo({ id: 900, text: 'From Matrix' });
  api.pushEvent('TodoCreated', {
    id: 900,
    room_id: '!general:mbm.one',
    owner_id: 'acct_00000000000000000000000000000001',
    sender_name: 'evan',
    text: 'From Matrix',
    priority: 'medium',
    due_at: null,
    remind_before_minutes: null,
    completed: false,
    created_at: new Date().toISOString(),
  });

  await expect(todoRow(app, 'From Matrix')).toBeVisible({ timeout: 20_000 });
});

// The fake answers each `/events` connection with the queued frames and then
// *ends the response*, so `EventSource` reconnects with backoff. That keeps the
// suite deterministic — no socket held open for the run — but it means the
// "connected" indicator only flickers, and asserting on it would measure the
// fake's timing rather than the app. Delivery is the thing worth asserting, and
// the specs above and below do that.

test('an unknown event type is ignored, and later events still arrive', async ({ app, api }) => {
  const errors: string[] = [];
  app.on('pageerror', (e) => errors.push(e.message));

  await gotoApp(app, TAB.todo);

  // A variant this frontend has no handler for — a newer backend, say. It must
  // not throw, and it must not poison the stream for what follows.
  api.pushEvent('SomethingNobodyHandles', { id: 1 });

  api.seedTodo({ id: 901, text: 'After the unknown one' });
  api.pushEvent('TodoCreated', {
    id: 901,
    room_id: '!general:mbm.one',
    owner_id: 'acct_00000000000000000000000000000001',
    sender_name: 'evan',
    text: 'After the unknown one',
    priority: 'medium',
    due_at: null,
    remind_before_minutes: null,
    completed: false,
    created_at: new Date().toISOString(),
  });

  await expect(todoRow(app, 'After the unknown one')).toBeVisible({ timeout: 20_000 });
  expect(errors).toEqual([]);
});

test('in-app toast notifications appear for ReminderTriggered, MorningBriefingSent, and PortfolioAlertTriggered', async ({
  app,
  api,
}) => {
  await gotoApp(app, TAB.todo);

  api.pushEvent('ReminderTriggered', {
    id: 77,
    room_id: 'web',
    owner_id: 'acct_00000000000000000000000000000001',
    message: 'Take a walk outside',
  });
  api.pushEvent('MorningBriefingSent', {
    owner_id: 'acct_00000000000000000000000000000001',
    summary: '2 tasks, 1 habit, 0 bills, 1 holding',
  });
  api.pushEvent('PortfolioAlertTriggered', {
    id: 12,
    owner_id: 'acct_00000000000000000000000000000001',
    symbol: 'NVDA',
  });

  await expect(app.getByText('Reminder Alert', { exact: true })).toBeVisible({ timeout: 20_000 });
  await expect(app.getByText('Take a walk outside')).toBeVisible();
  await expect(app.getByText('Morning Briefing', { exact: true })).toBeVisible();
  await expect(app.getByText('2 tasks, 1 habit, 0 bills, 1 holding')).toBeVisible();
  await expect(app.getByText('Price Alert Triggered', { exact: true })).toBeVisible();
  await expect(app.getByText('Price alert triggered for NVDA.')).toBeVisible();
});
