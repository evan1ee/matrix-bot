import { test, expect, gotoApp } from './fixtures';
import type { Page } from '@playwright/test';

/** Opens Settings → Integrations, where the calendar feed panel lives. */
async function openIntegrations(page: Page) {
  await gotoApp(page);
  await page.locator('button[title="Settings"]:visible').first().click();
  const dialog = page.getByRole('dialog', { name: /settings/i }).first();
  await dialog.getByRole('button', { name: /^integrations$/i }).click();
  return dialog;
}

test.describe('calendar feed', () => {
  test('creates a link and offers copy and a webcal subscribe action', async ({ app }) => {
    const dialog = await openIntegrations(app);
    await dialog.getByRole('button', { name: 'Create calendar link' }).click();

    const url = dialog.getByLabel('Subscription link');
    await expect(url).toHaveValue(/\/api\/v1\/calendar\/feed\.ics\?token=mb_cal_/);
    const subscribe = dialog.getByRole('link', { name: 'Subscribe in calendar app' });
    await expect(subscribe).toHaveAttribute('href', /^webcal:\/\/.+feed\.ics\?token=mb_cal_/);
    await expect(dialog.getByText(/Link active since/)).toBeVisible();
  });

  test('shows the link as active but never reveals the token after a reload', async ({ app, api }) => {
    api.calendarTokenCreatedAt = '2026-10-01T08:00:00Z';
    const dialog = await openIntegrations(app);

    await expect(dialog.getByText(/Link active since/)).toBeVisible();
    await expect(dialog.getByLabel('Subscription link')).toHaveCount(0);
    await expect(dialog.getByRole('button', { name: 'Create calendar link' })).toHaveCount(0);
  });

  test('regenerating asks first, then replaces the link', async ({ app, api }) => {
    api.calendarTokenCreatedAt = '2026-10-01T08:00:00Z';
    const dialog = await openIntegrations(app);

    await dialog.getByRole('button', { name: 'Regenerate link' }).click();
    const confirm = app.getByRole('alertdialog');
    await expect(confirm).toContainText('stops working immediately');

    // Cancel backs out without minting.
    await confirm.getByRole('button', { name: 'Cancel' }).click();
    await expect(confirm).toHaveCount(0);
    expect(api.calendarMintCount).toBe(0);

    await dialog.getByRole('button', { name: 'Regenerate link' }).click();
    await app.getByRole('alertdialog').getByRole('button', { name: 'Regenerate' }).click();
    await expect(dialog.getByLabel('Subscription link')).toHaveValue(/mb_cal_fake_token_1/);
    expect(api.calendarMintCount).toBe(1);
  });

  test('revoking returns to the empty state', async ({ app, api }) => {
    api.calendarTokenCreatedAt = '2026-10-01T08:00:00Z';
    const dialog = await openIntegrations(app);

    await dialog.getByRole('button', { name: 'Revoke link' }).click();
    await app.getByRole('alertdialog').getByRole('button', { name: 'Revoke' }).click();

    await expect(dialog.getByRole('button', { name: 'Create calendar link' })).toBeVisible();
    expect(api.calendarTokenCreatedAt).toBeNull();
  });
});
