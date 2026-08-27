import { test, expect, gotoApp, TAB, DASHBOARD_READY } from './fixtures';
import type { Page } from '@playwright/test';

/**
 * Captures every screen the suite drives, as a visual inventory.
 *
 * Not an assertion test — nothing here can fail meaningfully — so it is tagged
 * `@screens` and excluded from the normal run. Its job is to make the UI
 * reviewable in one place, including the states that are otherwise hard to
 * reach by hand: a pending merge, a recovery countdown, recovery codes shown
 * once.
 *
 *   npx playwright test e2e/screens.spec.ts --grep @screens
 */

const OUT = 'screenshots';

async function shot(page: Page, name: string) {
  await page.waitForTimeout(350); // let transitions settle
  await page.screenshot({ path: `${OUT}/${name}.png`, fullPage: false });
}

async function openSecurity(page: Page) {
  await page.locator('button[title="Settings"]:visible').first().click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('button', { name: /^security$/i }).click();
  return dialog;
}

test('@screens signed out and registration', async ({ anonApp }) => {
  await anonApp.goto('/');
  const dialog = anonApp.getByRole('dialog');
  await expect(dialog).toBeVisible();
  await shot(anonApp, '01-sign-in');

  await dialog.getByRole('button', { name: /create an account instead/i }).click();
  await shot(anonApp, '02-register');
});

test('@screens the module views', async ({ app, api }) => {
  api.seedTodo({ text: 'Renew passport', priority: 'high' });
  api.seedTodo({ text: 'Book dentist' });
  api.seedReminder({ message: 'Stand-up' });
  api.seedWebhook({
    name: 'Production VPS Health',
    default_room_id: '!ops:mbm.one',
    total_delivered_count: 42,
    failure_count: 0,
    last_triggered_at: '2026-08-27T03:00:00Z',
  });
  api.seedWebhook({
    name: 'GitHub CI/CD Alerts',
    default_room_id: '!general:mbm.one',
    total_delivered_count: 15,
    failure_count: 1,
    last_triggered_at: '2026-08-27T02:30:00Z',
  });

  await gotoApp(app);
  await expect(app.getByText(DASHBOARD_READY)).toBeVisible();
  await shot(app, '03-dashboard');

  for (const [n, tab] of [
    ['04-tasks', TAB.todo],
    ['05-reminders', TAB.reminder],
    ['06-expenses', TAB.finance],
    ['07-portfolio', TAB.portfolio],
    ['08-bills', TAB.bills],
    ['09-webhooks', TAB.webhooks],
  ] as const) {
    await app.getByRole('button', { name: tab, exact: true }).click();
    await shot(app, n);
  }
});

test('@screens webhook create and secret modal', async ({ app }) => {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.webhooks, exact: true }).click();
  await app.getByRole('button', { name: /create webhook/i }).first().click();
  await app.locator('input[placeholder*="VPS Health Monitor"]').fill('Production VPS Alerts');
  await app.getByRole('button', { name: 'Generate Webhook' }).click();
  await expect(app.getByText('Webhook Secret Generated')).toBeVisible();
  await shot(app, '10-webhook-secret-modal');
});

test('@screens webhook delivery logs viewer', async ({ app, api }) => {
  const hook = api.seedWebhook({ name: 'Production VPS Health' });
  api.seedWebhookLog(hook.id, {
    title: 'High CPU Usage Alert',
    plain_body: 'CPU load average exceeded 90% (current: 96.4%) on node vps-prod-01 for 5m.',
    severity: 'critical',
    status: 'delivered',
  });
  api.seedWebhookLog(hook.id, {
    title: 'Nightly Database Backup',
    plain_body: 'Backup database archived: snapshot-2026-08-27.tar.gz (size: 4.2GB).',
    severity: 'success',
    status: 'delivered',
  });
  api.seedWebhookLog(hook.id, {
    title: 'Deployment Warning',
    plain_body: 'Slow database query detected: SELECT * FROM transactions WHERE ... (took 3200ms)',
    severity: 'warning',
    status: 'delivered',
  });

  await gotoApp(app);
  await app.getByRole('button', { name: TAB.webhooks, exact: true }).click();
  await app.getByRole('button', { name: /history/i }).first().click();
  await expect(app.getByText(/Delivery Logs:/i)).toBeVisible();
  await shot(app, '11-webhook-logs-history');
});

test('@screens settings, both tabs', async ({ app }) => {
  await gotoApp(app);
  await app.locator('button[title="Settings"]:visible').first().click();
  await shot(app, '09-settings-preferences');

  await app.getByRole('dialog').getByRole('button', { name: /^security$/i }).click();
  await shot(app, '10-settings-security');
});

test('@screens security states that are hard to reach by hand', async ({ app, api }) => {
  api.passkeys = [
    { id: 'cred_1', label: 'MacBook Touch ID', created_at: '2026-01-01T10:00:00Z', last_used_at: '2026-08-11T09:00:00Z' },
    { id: 'cred_2', label: 'YubiKey 5C', created_at: '2026-02-02T10:00:00Z', last_used_at: null },
  ];
  api.linkedMxids = ['@evan:mbm.one'];
  api.sessions = [
    { id: 'a', created_at: '2026-08-01T10:00:00Z', last_seen_at: '2026-08-12T09:00:00Z', user_agent: 'Chrome on macOS', current: true },
    { id: 'b', created_at: '2026-07-01T10:00:00Z', last_seen_at: '2026-08-10T09:00:00Z', user_agent: 'Firefox on Linux', current: false },
  ];
  api.securityEvents = [
    { id: 3, kind: 'matrix_linked', detail: '@evan:mbm.one', created_at: '2026-08-11T09:00:00Z' },
    { id: 2, kind: 'passkey_added', detail: 'YubiKey 5C', created_at: '2026-08-10T09:00:00Z' },
    { id: 1, kind: 'signed_in', detail: null, created_at: '2026-08-10T08:00:00Z' },
  ];

  await gotoApp(app);
  let dialog = await openSecurity(app);
  await shot(app, '11-security-populated');

  await dialog.getByRole('button', { name: /link a matrix account/i }).click();
  await shot(app, '12-link-code');

  await dialog.getByRole('button', { name: /generate new codes/i }).click();
  await shot(app, '13-recovery-codes-shown-once');
});

test('@screens the two alarms', async ({ app, api }) => {
  api.recoveryStatus = { codes_remaining: 7, recovery_valid_after: '2026-08-13T09:00:00Z' };
  api.pendingMerge = {
    from_account: 'acct_old',
    preview: { todos: 47, reminders: 8, transactions: 132, recurring_bills: 0,
               portfolio_holdings: 3, portfolio_cash_balances: 1,
               finance_categories: 2, user_settings: 1 },
  };

  await gotoApp(app);
  await openSecurity(app);
  await shot(app, '14-recovery-alarm-and-pending-merge');
});

// The app defaults to DARK, so toggling lands on light. Named for what it
// produces rather than what the button says.
test('@screens the other theme', async ({ app }) => {
  await gotoApp(app);
  await app.locator('button[title*="light mode" i]:visible, button[title*="dark mode" i]:visible').first().click();
  await shot(app, '15-dashboard-light');
});

test.describe('mobile', () => {
  // The viewport the suite otherwise never renders — `MobileTopBar.tsx` and the
  // mobile sidebar exist and are exercised by nothing.
  test.use({ viewport: { width: 390, height: 844 } });

  test('@screens mobile shell', async ({ app }) => {
    await gotoApp(app);
    await shot(app, '16-mobile-dashboard');
  });
});
