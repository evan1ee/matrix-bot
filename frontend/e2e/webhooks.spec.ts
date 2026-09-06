import { test, expect, gotoApp, TAB } from './fixtures';

test('navigates to webhooks and displays empty state', async ({ app }) => {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.webhooks, exact: true }).click();
  await expect(app.getByRole('heading', { name: 'Inbound Webhooks', exact: true })).toBeVisible();
  await expect(app.getByText(/No inbound webhooks configured/i)).toBeVisible();
});

test('creates a new webhook and displays secret token modal', async ({ app, api }) => {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.webhooks, exact: true }).click();

  await app.getByRole('button', { name: /create webhook/i }).first().click();
  await expect(app.getByText('Create Inbound Webhook')).toBeVisible();

  await app.locator('input[placeholder*="VPS Health Monitor"]').fill('Production Alerts');
  await app.getByRole('button', { name: 'Generate Webhook' }).click();

  // Secret display modal appears
  await expect(app.getByText('Webhook Secret Generated')).toBeVisible();
  await expect(app.getByText(/Copy this secret token now/i)).toBeVisible();
  await expect(app.locator('input[value*="whk_"]')).toBeVisible();

  await app.getByRole('button', { name: 'I Have Saved My Secret' }).click();

  // Webhook card appears in list
  await expect(app.getByText('Production Alerts')).toBeVisible();
  await expect(app.getByText('Active')).toBeVisible();
});

test('sends a test ping for a webhook', async ({ app, api }) => {
  api.seedWebhook({ name: 'CI/CD Runner', default_room_id: '!ops:mbm.one' });
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.webhooks, exact: true }).click();

  await expect(app.getByText('CI/CD Runner')).toBeVisible();
  await app.getByRole('button', { name: /test ping/i }).click();

  await expect(app.getByText(/Test alert queued successfully/i)).toBeVisible();
});

test('rotates webhook secret token and deletes webhook', async ({ app, api }) => {
  const hook = api.seedWebhook({ name: 'Legacy Hook' });
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.webhooks, exact: true }).click();

  await app.getByRole('button', { name: /rotate secret/i }).click();
  const rotateModal = app.locator('[data-testid="confirm-modal"]');
  await expect(rotateModal).toBeVisible();
  await rotateModal.getByRole('button', { name: 'Rotate Secret' }).click();

  await expect(app.getByText('Webhook Secret Generated')).toBeVisible();
  await app.getByRole('button', { name: 'I Have Saved My Secret' }).click();

  // Delete
  await app.locator('button[title="Delete webhook"]').click();
  const deleteModal = app.locator('[data-testid="confirm-modal"]');
  await expect(deleteModal).toBeVisible();
  await deleteModal.getByRole('button', { name: 'Delete Webhook' }).click();
  await expect(deleteModal).not.toBeVisible();
  await expect(app.getByRole('heading', { name: 'Legacy Hook' })).not.toBeVisible();
});

test('opens webhook history and inspects delivery logs', async ({ app, api }) => {
  const hook = api.seedWebhook({ name: 'Database Monitor' });
  api.seedWebhookLog(hook.id, {
    title: 'High Memory Usage',
    plain_body: 'RAM consumption reached 94% on worker node 2.',
    severity: 'critical',
    status: 'delivered',
  });
  api.seedWebhookLog(hook.id, {
    title: 'Scheduled Backup Done',
    plain_body: 'Daily database backup saved to S3 bucket.',
    severity: 'success',
    status: 'delivered',
  });

  await gotoApp(app);
  await app.getByRole('button', { name: TAB.webhooks, exact: true }).click();

  await app.getByRole('button', { name: /history/i }).click();
  await expect(app.getByText('Delivery Logs: Database Monitor')).toBeVisible();
  await expect(app.getByText('High Memory Usage')).toBeVisible();
  await expect(app.getByText(/RAM consumption reached 94%/i)).toBeVisible();
  await expect(app.getByText('Scheduled Backup Done')).toBeVisible();

  await app.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(app.getByText('Delivery Logs: Database Monitor')).not.toBeVisible();
});

