import { test, expect, gotoApp, TAB } from './fixtures';

test('hides admin navigation link for non-admin users and shows it for admins', async ({ app, api }) => {
  api.role = 'user';
  await gotoApp(app);
  await expect(app.getByRole('button', { name: TAB.admin, exact: true })).toHaveCount(0);

  api.role = 'admin';
  await app.reload();
  await expect(app.getByRole('button', { name: TAB.admin, exact: true })).toBeVisible();
});

test('displays system status overview in Admin Console', async ({ app, api }) => {
  api.role = 'admin';
  await gotoApp(app, TAB.admin);

  await expect(app.getByRole('heading', { name: 'Admin Console', exact: true }).first()).toBeVisible();
  await expect(app.getByText('Encryption Active')).toBeVisible();
  await expect(app.getByText('Runtime Settings')).toBeVisible();
  await expect(app.getByText('Encrypted Credentials', { exact: true })).toBeVisible();
  await expect(app.getByText('AES-256-GCM')).toBeVisible();
});

test('edits and resets a runtime setting and records audit log', async ({ app, api }) => {
  api.role = 'admin';
  await gotoApp(app, TAB.admin);

  // Switch to LLM & Models tab
  await app.getByRole('button', { name: 'LLM & Models' }).click();
  await expect(app.getByText('llm.default_model')).toBeVisible();
  await expect(app.getByText('gpt-4o-mini').first()).toBeVisible();

  // Edit llm.default_model
  const row = app.locator('tr', { hasText: 'llm.default_model' });
  await row.getByRole('button', { name: 'Edit' }).click();

  await expect(app.getByRole('heading', { name: 'Edit Setting' })).toBeVisible();
  const input = app.locator('form input[type="text"]');
  await input.fill('claude-3-7-sonnet');
  await app.getByRole('button', { name: 'Save Setting' }).click();

  // Verify updated value and custom badge
  await expect(row.getByText('claude-3-7-sonnet')).toBeVisible();
  await expect(row.getByText('custom')).toBeVisible();

  // Verify audit log recorded the change
  await app.getByRole('button', { name: 'Audit Log' }).click();
  await expect(app.getByText('setting_updated')).toBeVisible();
  await expect(app.getByText('llm.default_model')).toBeVisible();

  // Switch back to LLM & Models and reset setting to default
  await app.getByRole('button', { name: 'LLM & Models' }).click();
  await row.locator('button[title="Reset to default"]').click();
  const confirmModal = app.locator('[data-testid="confirm-modal"]');
  await expect(confirmModal).toBeVisible();
  await confirmModal.getByRole('button', { name: 'Reset' }).click();

  await expect(row.getByText('custom')).toHaveCount(0);
});

test('stores write-only credential, tests connectivity, and never leaks plaintext in DOM', async ({ app, api }) => {
  api.role = 'admin';
  await gotoApp(app, TAB.admin);

  await app.getByRole('button', { name: 'Secrets Vault' }).click();
  await expect(app.getByText(/No external credentials currently configured/i)).toBeVisible();

  // Open Add Credential modal
  await app.getByRole('button', { name: /add credential/i }).click();
  await expect(app.getByRole('heading', { name: 'Add Encrypted Credential' })).toBeVisible();

  const rawSecret = 'sk-super-secret-plaintext-value-9999';
  const secretInput = app.getByLabel('Secret Value');
  await expect(secretInput).toHaveAttribute('type', 'password');
  await secretInput.fill(rawSecret);
  await app.getByRole('button', { name: 'Store Secret' }).click();

  // Verify masked preview is shown and plaintext never appears in DOM
  await expect(app.getByText('sk-...9999')).toBeVisible();
  const bodyContent = await app.content();
  expect(bodyContent).not.toContain(rawSecret);

  // Test connectivity
  const secretRow = app.locator('tr', { hasText: 'sk-...9999' });
  await secretRow.locator('button[title="Test Connectivity"]').click();
  await expect(secretRow.getByText('healthy')).toBeVisible();

  // Delete credential
  await secretRow.locator('button[title="Delete Credential"]').click();
  const deleteModal = app.locator('[data-testid="confirm-modal"]');
  await expect(deleteModal).toBeVisible();
  await deleteModal.getByRole('button', { name: 'Delete' }).click();
  await expect(app.getByText('sk-...9999')).toHaveCount(0);
});
