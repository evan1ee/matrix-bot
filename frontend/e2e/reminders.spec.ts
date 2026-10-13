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

test('displays reminder history with status badges and filters', async ({ app, api }) => {
  api.seedReminder({ message: 'Pending task', status: 'pending' });
  api.seedReminder({ message: 'Triggered task', status: 'triggered' });
  api.seedReminder({ message: 'Cancelled task', status: 'cancelled' });
  await gotoApp(app, TAB.reminder);

  // Pending section has Pending task
  await expect(app.getByText('Pending task')).toBeVisible();

  // History section has Triggered and Cancelled tasks
  await expect(app.getByText('Reminder history')).toBeVisible();
  await expect(app.getByText('Triggered task')).toBeVisible();
  await expect(app.getByText('Cancelled task')).toBeVisible();

  // Filter to only Triggered
  await app.getByRole('button', { name: 'Triggered' }).click();
  await expect(app.getByText('Triggered task')).toBeVisible();
  await expect(app.getByText('Cancelled task')).toHaveCount(0);

  // Filter to Cancelled
  await app.getByRole('button', { name: 'Cancelled' }).click();
  await expect(app.getByText('Cancelled task')).toBeVisible();
  await expect(app.getByText('Triggered task')).toHaveCount(0);
});

test('deletes an item from history', async ({ app, api }) => {
  api.seedReminder({ message: 'Historical item', status: 'cancelled' });
  await gotoApp(app, TAB.reminder);

  await expect(app.getByText('Historical item')).toBeVisible();
  await app.getByTitle('Delete from history').click();

  await expect(app.getByText('Historical item')).toHaveCount(0);
  expect(api.reminders.find((r) => r.message === 'Historical item')).toBeUndefined();
});

test('clears all reminder history', async ({ app, api }) => {
  api.seedReminder({ message: 'Keep me pending', status: 'pending' });
  api.seedReminder({ message: 'History 1', status: 'triggered' });
  api.seedReminder({ message: 'History 2', status: 'cancelled' });
  await gotoApp(app, TAB.reminder);

  await expect(app.getByText('History 1')).toBeVisible();
  await expect(app.getByText('History 2')).toBeVisible();

  await app.getByRole('button', { name: 'Clear history' }).click();
  await app.getByRole('button', { name: 'Yes, clear' }).click();

  await expect(app.getByText('No reminder history found.')).toBeVisible();
  await expect(app.getByText('Keep me pending')).toBeVisible();
  expect(api.reminders.filter((r) => r.status !== 'pending')).toHaveLength(0);
});

test('re-schedules an item from history into form', async ({ app, api }) => {
  api.seedReminder({ message: 'Weekly sync meeting', recurrence: 'weekly', status: 'triggered' });
  await gotoApp(app, TAB.reminder);

  await expect(app.getByText('Weekly sync meeting')).toBeVisible();
  await app.getByTitle('Re-schedule this reminder').click();

  await expect(app.getByPlaceholder('Reminder message...')).toHaveValue('Weekly sync meeting');
});

test('formats raw acct_ sender IDs gracefully with user display name', async ({ app, api }) => {
  api.seedReminder({
    message: 'Have my hair cut',
    sender_name: 'acct_796cac95ea66412e8d758526fab863cd',
    status: 'pending',
  });
  await gotoApp(app, TAB.reminder);

  await expect(app.getByText('Have my hair cut')).toBeVisible();
  await expect(app.getByText('by acct_796cac95ea66412e8d758526fab863cd')).toHaveCount(0);
  await expect(app.getByText('by tester')).toBeVisible();
});

test('creates a web (in-app) reminder when no external chat rooms are linked', async ({
  app,
  api,
}) => {
  api.rooms = [];
  await gotoApp(app, TAB.reminder);

  await expect(app.getByRole('button', { name: 'Web (in-app)' })).toBeVisible();
  await app.getByPlaceholder('Reminder message...').fill('Stretch in-app');
  await app.getByRole('button', { name: 'Set Reminder' }).click();

  await expect(app.getByText('Stretch in-app')).toBeVisible();
  expect(api.reminders).toHaveLength(1);
  expect(api.reminders[0].room_id).toBe('web');
});
