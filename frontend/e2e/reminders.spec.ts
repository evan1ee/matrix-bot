import { test, expect, gotoApp, TAB } from './fixtures';

test('lists pending reminders', async ({ app, api }) => {
  api.seedReminder({ message: 'Water the plants' });
  api.seedReminder({ message: 'Stand-up call' });
  await gotoApp(app, TAB.reminder);

  await expect(app.getByText('Water the plants')).toBeVisible();
  await expect(app.getByText('Stand-up call')).toBeVisible();
});

test('creates a reminder', async ({ app, api }) => {
  await gotoApp(app, TAB.reminder);

  await app.getByPlaceholder('Reminder message...').fill('Call the bank');
  await app.getByRole('button', { name: 'Set Reminder' }).click();

  await expect(app.getByText('Call the bank')).toBeVisible();
  expect(api.reminders.map((r) => r.message)).toContain('Call the bank');
});

test('will not create an empty reminder', async ({ app, api }) => {
  await gotoApp(app, TAB.reminder);

  await app.getByRole('button', { name: 'Set Reminder' }).click();

  expect(api.reminders).toHaveLength(0);
});

test('cancels a reminder', async ({ app, api }) => {
  api.seedReminder({ message: 'Cancel me' });
  await gotoApp(app, TAB.reminder);

  await app.getByTitle('Cancel reminder').first().click();

  await expect(app.getByText('Cancel me')).toHaveCount(0);
  await expect.poll(() => api.reminders[0].status).toBe('cancelled');
});

test('a reminder scheduled from a todo shows up in the reminders list', async ({ app, api }) => {
  const todo = api.seedTodo({
    text: 'Ship the thing',
    due_at: new Date(Date.now() + 6 * 3_600_000).toISOString(),
    remind_before_minutes: 60,
  });
  api.seedReminder({
    todo_id: todo.id,
    message: `Task #${todo.id} is due: ${todo.text}`,
    room_id: '!ops:mbm.one',
  });
  await gotoApp(app, TAB.reminder);

  await expect(app.getByText(`Task #${todo.id} is due: Ship the thing`)).toBeVisible();
});
