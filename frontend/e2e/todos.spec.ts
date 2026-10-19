import {
  test,
  expect,
  gotoApp,
  todoRow,
  chooseOption,
  dateField,
  setDateTime,
  datetimeLocal,
  TAB,
} from './fixtures';

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

test('creates a tagged todo and filters by tag', async ({ app, api }) => {
  api.seedTodo({ text: 'Untagged chore' });
  await gotoApp(app, TAB.todo);

  await app.getByPlaceholder('What needs to be done?').fill('Ship the report');
  const tags = app.getByLabel('Tags');
  await tags.fill('work');
  await tags.press('Enter');
  await tags.fill('#Q4,');
  await app.getByRole('button', { name: 'Add Task' }).click();

  const row = todoRow(app, 'Ship the report');
  await expect(row).toBeVisible();
  await expect(row.getByText('#work')).toBeVisible();
  await expect(row.getByText('#q4')).toBeVisible();
  expect(api.todos.find((t) => t.text === 'Ship the report')?.tags).toEqual(['q4', 'work']);

  await app.getByLabel('Filter by tag').selectOption('work');
  await expect(todoRow(app, 'Ship the report')).toBeVisible();
  await expect(todoRow(app, 'Untagged chore')).toHaveCount(0);
});

test('repeat selector requires a due date', async ({ app, api }) => {
  await gotoApp(app, TAB.todo);
  const repeat = app.getByLabel('Repeat');
  await expect(repeat).toBeDisabled();

  await setDateTime(dateField(app, 'Due date'), datetimeLocal(6 * 60));
  await expect(repeat).toBeEnabled();
  await repeat.selectOption('weekly');

  await app.getByPlaceholder('What needs to be done?').fill('Water plants');
  await app.getByRole('button', { name: 'Add Task' }).click();

  const row = todoRow(app, 'Water plants');
  await expect(row.getByText('↻ weekly')).toBeVisible();
  expect(api.todos.find((t) => t.text === 'Water plants')?.recurrence).toBe('weekly');
});

test('completing a recurring todo shows its successor', async ({ app, api }) => {
  const due = new Date(Date.now() + 3600_000).toISOString();
  api.seedTodo({ text: 'Take vitamins', due_at: due, recurrence: 'daily' });
  await gotoApp(app, TAB.todo);

  await todoRow(app, 'Take vitamins').getByRole('button').first().click();

  await expect(todoRow(app, 'Take vitamins')).toHaveCount(2);
  const [done, next] = api.todos;
  expect(done.completed).toBe(true);
  expect(done.next_todo_id).toBe(next.id);
  expect(next.completed).toBe(false);
  expect(new Date(next.due_at!).getTime() - new Date(due).getTime()).toBe(86_400_000);
});

test('checks off and reorders steps', async ({ app, api }) => {
  const todo = api.seedTodo({ text: 'Plan the trip' });
  await gotoApp(app, TAB.todo);

  const row = todoRow(app, 'Plan the trip');
  await row.getByRole('button', { name: 'Show checklist' }).click();
  const input = app.getByLabel('New step');
  await input.fill('Book flights');
  await input.press('Enter');
  await input.fill('Pack bags');
  await input.press('Enter');
  await expect(app.getByText('Pack bags')).toBeVisible();
  expect(todo.steps.map((s) => s.text)).toEqual(['Book flights', 'Pack bags']);

  await app.getByRole('checkbox', { name: 'Book flights' }).click();
  await expect.poll(() => todo.steps[0].done).toBe(true);
  await expect(row.getByText('1/2')).toBeVisible();

  await app.getByRole('button', { name: 'Move Book flights down' }).click();
  await expect.poll(() => todo.steps.map((s) => s.text)).toEqual(['Pack bags', 'Book flights']);

  await app.getByRole('button', { name: 'Remove Pack bags' }).click();
  await expect.poll(() => todo.steps.length).toBe(1);
});

test('rejects a step over 200 characters', async ({ app, api }) => {
  api.seedTodo({ text: 'Long step' });
  await gotoApp(app, TAB.todo);
  await todoRow(app, 'Long step').getByRole('button', { name: 'Show checklist' }).click();
  const input = app.getByLabel('New step');
  await input.fill('x'.repeat(201));
  await input.press('Enter');
  await expect(app.getByRole('alert')).toContainText('200');
});
