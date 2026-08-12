import { test, expect, gotoApp, DASHBOARD_READY, TAB } from './fixtures';

test('switches between module tabs', async ({ app }) => {
  await gotoApp(app);

  // The dashboard is the landing view. It used to be the `core` module's
  // "System Event Feed" — the redesigned sidebar hardcodes its own nav list and
  // no longer offers `core` at all, so that view is unreachable from the UI.
  await expect(app.getByText(DASHBOARD_READY)).toBeVisible();

  await app.getByRole('button', { name: TAB.todo, exact: true }).click();
  await expect(app.getByText('Create New Task')).toBeVisible();

  await app.getByRole('button', { name: TAB.reminder, exact: true }).click();
  await expect(app.getByText('Schedule Reminder')).toBeVisible();

  await app.getByRole('button', { name: TAB.finance, exact: true }).click();
  await expect(app.getByText('Record Entry')).toBeVisible();

  // And back again — switching away and returning must not lose the view.
  await app.getByRole('button', { name: TAB.todo, exact: true }).click();
  await expect(app.getByText('Create New Task')).toBeVisible();
});

test('shows live counts in the header stats', async ({ app, api }) => {
  api.seedTodo({ text: 'One' });
  api.seedTodo({ text: 'Two' });
  api.seedReminder({ message: 'Ping' });
  await gotoApp(app);

  // Each stat card carries its label in `data-stat`. Located that way rather
  // than by walking up from the label text: the card is a <button>, so the old
  // `ancestor::div[...]` step matched nothing and the assertion could never
  // have passed whatever the counts were.
  const statCard = (label: string) => app.locator(`[data-testid="stat-card"][data-stat="${label}"]`);

  // The raw label, not what is rendered — the card upper-cases it in CSS.
  await expect(statCard('Active Tasks')).toContainText('2');
  await expect(statCard('Reminders')).toContainText('1');
});

// The backend-health panel lived in the `core` module, which the redesigned
// sidebar no longer offers — there is no way to reach it from the UI, so there
// is nothing for a browser test to assert. `/health` is covered by the Rust
// tests. What the dashboard does surface is whether the live stream is up, and
// that is worth holding onto.
test('shows the live-stream connection state', async ({ app }) => {
  await gotoApp(app);

  await expect(app.getByText(/SSE Live|Reconnecting|Auth Required/i).first()).toBeVisible();
});

test('persists the chosen theme across reloads', async ({ app }) => {
  await gotoApp(app);

  const before = await app.evaluate(() => document.documentElement.classList.contains('dark'));

  // The theme control is the only button in the header carrying a theme title.
  await app.getByRole('button', { name: /theme|dark|light|system/i }).first().click();
  const options = app.getByRole('button', { name: before ? /^light$/i : /^dark$/i });
  if (await options.count()) {
    await options.first().click();
  }

  await expect
    .poll(() => app.evaluate(() => document.documentElement.classList.contains('dark')))
    .not.toBe(before);

  await app.reload();
  await expect(app.getByText(DASHBOARD_READY)).toBeVisible();
  expect(await app.evaluate(() => document.documentElement.classList.contains('dark'))).not.toBe(
    before
  );
});
