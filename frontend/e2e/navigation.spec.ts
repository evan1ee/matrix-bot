import { test, expect, gotoApp } from './fixtures';

test('switches between module tabs', async ({ app }) => {
  await gotoApp(app);

  // The dashboard is the landing view.
  await expect(app.getByText('System Event Feed')).toBeVisible();

  await app.getByRole('button', { name: 'Todos', exact: true }).click();
  await expect(app.getByText('Create New Task')).toBeVisible();

  await app.getByRole('button', { name: 'Reminders', exact: true }).click();
  await expect(app.getByText('Schedule Reminder')).toBeVisible();
});

test('shows live counts in the header stats', async ({ app, api }) => {
  api.seedTodo({ text: 'One' });
  api.seedTodo({ text: 'Two' });
  api.seedReminder({ message: 'Ping' });
  await gotoApp(app);

  // Each stat is a card wrapping the label and the count; the label itself is a
  // leaf <span>, so step up to the card that holds both.
  const statCard = (label: string) =>
    app.getByText(label, { exact: true }).locator('xpath=ancestor::div[contains(@class,"rounded-lg")][1]');

  await expect(statCard('TOTAL TASKS')).toContainText('2');
  await expect(statCard('ACTIVE REMINDERS')).toContainText('1');
});

test('reports backend health on the dashboard', async ({ app }) => {
  await gotoApp(app);

  await expect(app.getByText('REST API SERVICE')).toBeVisible();
  await expect(app.getByText('SQLITE DATABASE')).toBeVisible();
  await expect(app.getByText('SSE BROADCASTER')).toBeVisible();
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
  await expect(app.getByText('TOTAL TASKS')).toBeVisible();
  expect(await app.evaluate(() => document.documentElement.classList.contains('dark'))).not.toBe(
    before
  );
});
