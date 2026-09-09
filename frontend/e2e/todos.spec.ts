import { test, expect, gotoApp, todoRow, chooseOption, TAB } from './fixtures';

test('creates a task and shows it in the list', async ({ app, api }) => {
  await gotoApp(app, TAB.todo);
  await app.getByPlaceholder('What needs to be done?').fill('Write the release notes');
  await app.getByRole('button', { name: 'Add Task' }).click();

  await expect(todoRow(app, 'Write the release notes')).toBeVisible();
  expect(api.todos.map((t) => t.text)).toContain('Write the release notes');
});

test('creates a task with a chosen priority', async ({ app, api }) => {
  await gotoApp(app, TAB.todo);
  await app.getByPlaceholder('What needs to be done?').fill('Urgent thing');
  await chooseOption(app.getByRole('button', { name: 'Medium Priority' }), 'High Priority');
  await app.getByRole('button', { name: 'Add Task' }).click();

  const row = todoRow(app, 'Urgent thing');
  await expect(row).toBeVisible();
  await expect(row.getByText('high')).toBeVisible();
  expect(api.todos.find((t) => t.text === 'Urgent thing')?.priority).toBe('high');
});

test('refuses to submit an empty task', async ({ app, api }) => {
  await gotoApp(app, TAB.todo);
  await app.getByRole('button', { name: 'Add Task' }).click();
  // Nothing is sent — the form guards before the request.
  expect(api.requests.filter((r) => r.method === 'POST' && r.url === '/todos')).toHaveLength(0);
});

test('rejects text past the 500 character limit', async ({ app, api }) => {
  await gotoApp(app, TAB.todo);
  await app.getByPlaceholder('What needs to be done?').fill('x'.repeat(501));
  await app.getByRole('button', { name: 'Add Task' }).click();

  await expect(app.getByText('Task text exceeds 500 character limit.')).toBeVisible();
  expect(api.todos).toHaveLength(0);
});

test('surfaces a server rejection instead of swallowing it', async ({ app, api }) => {
  await gotoApp(app, TAB.todo);
  api.failNext = { path: /^\/todos$/, status: 400, error: 'Task text required' };

  await app.getByPlaceholder('What needs to be done?').fill('Doomed task');
  await app.getByRole('button', { name: 'Add Task' }).click();

  await expect(app.getByText('Task text required')).toBeVisible();
});

test('toggles a task between active and completed', async ({ app, api }) => {
  api.seedTodo({ text: 'Toggle me' });
  await gotoApp(app, TAB.todo);

  const row = todoRow(app, 'Toggle me');
  await row.getByRole('button').first().click();

  await expect
    .poll(() => api.todos.find((t) => t.text === 'Toggle me')?.completed)
    .toBe(true);

  await todoRow(app, 'Toggle me').getByRole('button').first().click();
  await expect
    .poll(() => api.todos.find((t) => t.text === 'Toggle me')?.completed)
    .toBe(false);
});

test('deletes a task', async ({ app, api }) => {
  api.seedTodo({ text: 'Delete me' });
  await gotoApp(app, TAB.todo);

  await todoRow(app, 'Delete me').getByTitle('Delete task').click();

  await expect(todoRow(app, 'Delete me')).toHaveCount(0);
  expect(api.todos).toHaveLength(0);
});

test('filters by status', async ({ app, api }) => {
  api.seedTodo({ text: 'Still open' });
  api.seedTodo({ text: 'Already done', completed: true });
  await gotoApp(app, TAB.todo);

  await expect(todoRow(app, 'Still open')).toBeVisible();
  await expect(todoRow(app, 'Already done')).toBeVisible();

  await app.getByRole('button', { name: 'active', exact: true }).click();
  await expect(todoRow(app, 'Still open')).toBeVisible();
  await expect(todoRow(app, 'Already done')).toHaveCount(0);

  await app.getByRole('button', { name: 'completed', exact: true }).click();
  await expect(todoRow(app, 'Still open')).toHaveCount(0);
  await expect(todoRow(app, 'Already done')).toBeVisible();

  await app.getByRole('button', { name: 'all', exact: true }).click();
  await expect(todoRow(app, 'Still open')).toBeVisible();
  await expect(todoRow(app, 'Already done')).toBeVisible();
});

test('shows an empty state when there is nothing to show', async ({ app }) => {
  await gotoApp(app, TAB.todo);
  await expect(app.getByText('No tasks found. Create one above!')).toBeVisible();
});
