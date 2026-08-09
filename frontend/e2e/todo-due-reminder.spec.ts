import {
  test,
  expect,
  gotoApp,
  todoRow,
  todoEditForm,
  chooseOption,
  selectField,
  dateField,
  setDateTime,
  clearDateTime,
  datetimeLocal,
} from './fixtures';

const HOUR = 60;

test.describe('setting a due date and reminder at creation', () => {
  test('schedules a reminder in the chosen room', async ({ app, api }) => {
    await gotoApp(app, 'Todos');

    await app.getByPlaceholder('What needs to be done?').fill('Cut the release');
    await setDateTime(dateField(app, 'Due date'), datetimeLocal(6 * HOUR));
    await chooseOption(selectField(app, 'Remind me'), '1 hour before');
    await chooseOption(selectField(app, 'Send to room'), 'Ops');
    await app.getByRole('button', { name: 'Add Task' }).click();

    await expect(todoRow(app, 'Cut the release')).toBeVisible();

    const todo = api.todos.find((t) => t.text === 'Cut the release')!;
    expect(todo.remind_before_minutes).toBe(60);
    expect(todo.due_at).toBeTruthy();

    const reminder = api.reminderForTodo(todo.id)!;
    expect(reminder.room_id).toBe('!ops:mbm.one');
    expect(new Date(reminder.trigger_at).getTime()).toBe(
      new Date(todo.due_at!).getTime() - 60 * 60_000
    );
  });

  test('hides the room picker until a reminder is wanted', async ({ app }) => {
    await gotoApp(app, 'Todos');

    await expect(app.getByText('Send to room')).toHaveCount(0);
    await chooseOption(selectField(app, 'Remind me'), '1 day before');
    await expect(app.getByText('Send to room')).toBeVisible();
  });

  test('will not submit a reminder without a due date', async ({ app, api }) => {
    await gotoApp(app, 'Todos');

    await app.getByPlaceholder('What needs to be done?').fill('No date');
    await chooseOption(selectField(app, 'Remind me'), '1 hour before');
    await chooseOption(selectField(app, 'Send to room'), 'General');
    await app.getByRole('button', { name: 'Add Task' }).click();

    await expect(
      app.getByText('Pick a due date for the reminder to count back from.')
    ).toBeVisible();
    expect(api.todos).toHaveLength(0);
  });

  test('will not submit a reminder without a room', async ({ app, api }) => {
    await gotoApp(app, 'Todos');

    await app.getByPlaceholder('What needs to be done?').fill('No room');
    await setDateTime(dateField(app, 'Due date'), datetimeLocal(6 * HOUR));
    await chooseOption(selectField(app, 'Remind me'), '1 hour before');
    await app.getByRole('button', { name: 'Add Task' }).click();

    await expect(
      app.getByText('Choose a Matrix room for the reminder to be sent to.')
    ).toBeVisible();
    expect(api.todos).toHaveLength(0);
  });

  test('a due date alone needs no room', async ({ app, api }) => {
    await gotoApp(app, 'Todos');

    await app.getByPlaceholder('What needs to be done?').fill('Just a deadline');
    await setDateTime(dateField(app, 'Due date'), datetimeLocal(3 * HOUR));
    await app.getByRole('button', { name: 'Add Task' }).click();

    await expect(todoRow(app, 'Just a deadline')).toBeVisible();
    const todo = api.todos.find((t) => t.text === 'Just a deadline')!;
    expect(todo.due_at).toBeTruthy();
    expect(todo.remind_before_minutes).toBeNull();
    expect(api.reminderForTodo(todo.id)).toBeUndefined();
  });

  test('explains why a local account has no rooms to pick', async ({ app, api }) => {
    api.rooms = [];
    await gotoApp(app, 'Todos');

    await chooseOption(selectField(app, 'Remind me'), '1 hour before');
    await expect(app.getByText(/No Matrix rooms available/)).toBeVisible();
  });
});

test.describe('showing due dates in the list', () => {
  test('renders the due date and lead time', async ({ app, api }) => {
    api.seedTodo({
      text: 'Has a deadline',
      due_at: new Date('2030-06-15T14:30:00Z').toISOString(),
      remind_before_minutes: 60,
    });
    await gotoApp(app, 'Todos');

    const row = todoRow(app, 'Has a deadline');
    // The suite pins the browser to UTC, so the rendered local time is the
    // stored instant.
    await expect(row.getByText('Jun 15, 2030, 2:30 PM')).toBeVisible();
    await expect(row.getByText('1 hour before')).toBeVisible();
  });

  test('marks an overdue task', async ({ app, api }) => {
    api.seedTodo({
      text: 'Overdue task',
      due_at: new Date(Date.now() - 3_600_000).toISOString(),
    });
    api.seedTodo({
      text: 'Future task',
      due_at: new Date(Date.now() + 3_600_000).toISOString(),
    });
    await gotoApp(app, 'Todos');

    // Asserted via a data attribute rather than a Tailwind class: the class set
    // varies with the active theme, which made this assertion flaky.
    await expect(todoRow(app, 'Overdue task').getByTestId('due-date')).toHaveAttribute(
      'data-overdue',
      'true'
    );
    await expect(todoRow(app, 'Future task').getByTestId('due-date')).toHaveAttribute(
      'data-overdue',
      'false'
    );
  });
});

test.describe('editing an existing task', () => {
  test('edits text and priority without disturbing the reminder', async ({ app, api }) => {
    const todo = api.seedTodo({
      text: 'Original text',
      due_at: new Date(Date.now() + 6 * 3_600_000).toISOString(),
      remind_before_minutes: 60,
    });
    api.seedReminder({
      todo_id: todo.id,
      room_id: '!ops:mbm.one',
      message: `Task #${todo.id} is due: ${todo.text}`,
    });
    await gotoApp(app, 'Todos');

    await todoRow(app, 'Original text').getByTitle('Edit task').click();

    const form = todoEditForm(app);
    await form.locator('input[type="text"]').fill('Renamed task');
    await chooseOption(selectField(form, 'Priority'), 'High Priority');
    await form.getByRole('button', { name: 'Save' }).click();

    await expect(todoRow(app, 'Renamed task')).toBeVisible();
    expect(api.todos[0].text).toBe('Renamed task');
    expect(api.todos[0].priority).toBe('high');

    // The room was never restated, so the reminder must keep the one it had.
    const reminder = api.reminderForTodo(todo.id)!;
    expect(reminder.room_id).toBe('!ops:mbm.one');

    const patch = api.requests.find((r) => r.method === 'PATCH')!.body as Record<string, unknown>;
    expect(patch).not.toHaveProperty('remind_room_id');
  });

  test('adds a due date and reminder to a task that had none', async ({ app, api }) => {
    const todo = api.seedTodo({ text: 'Bare task' });
    await gotoApp(app, 'Todos');

    await todoRow(app, 'Bare task').getByTitle('Edit task').click();
    const form = todoEditForm(app);
    await setDateTime(dateField(form, 'Due date'), datetimeLocal(8 * HOUR));
    await chooseOption(selectField(form, 'Remind me'), '10 minutes before');
    await chooseOption(selectField(form, 'Send to room'), 'General');
    await form.getByRole('button', { name: 'Save' }).click();

    await expect(todoEditForm(app)).toHaveCount(0);
    await expect
      .poll(() => api.todos.find((t) => t.id === todo.id)?.remind_before_minutes)
      .toBe(10);
    expect(api.reminderForTodo(todo.id)?.room_id).toBe('!general:mbm.one');
  });

  test('requires a room when adding a first reminder to a web task', async ({ app, api }) => {
    api.seedTodo({ text: 'Needs a room' });
    await gotoApp(app, 'Todos');

    await todoRow(app, 'Needs a room').getByTitle('Edit task').click();
    const form = todoEditForm(app);
    await setDateTime(dateField(form, 'Due date'), datetimeLocal(8 * HOUR));
    await chooseOption(selectField(form, 'Remind me'), '1 hour before');
    await form.getByRole('button', { name: 'Save' }).click();

    await expect(
      form.getByText('Choose a Matrix room for the reminder to be sent to.')
    ).toBeVisible();
    expect(api.requests.some((r) => r.method === 'PATCH')).toBe(false);
  });

  test('changes the lead time and reschedules the reminder', async ({ app, api }) => {
    const due = new Date(Date.now() + 48 * 3_600_000);
    const todo = api.seedTodo({
      text: 'Reschedule me',
      due_at: due.toISOString(),
      remind_before_minutes: 60,
    });
    api.seedReminder({ todo_id: todo.id, room_id: '!general:mbm.one' });
    await gotoApp(app, 'Todos');

    await todoRow(app, 'Reschedule me').getByTitle('Edit task').click();
    const form = todoEditForm(app);
    await chooseOption(selectField(form, 'Remind me'), '1 day before');
    await form.getByRole('button', { name: 'Save' }).click();

    await expect(todoEditForm(app)).toHaveCount(0);
    await expect
      .poll(() => api.todos.find((t) => t.id === todo.id)?.remind_before_minutes)
      .toBe(1440);

    // The edit form round-trips through the picker, which has minute
    // granularity — so compare against the stored date, not the seeded one,
    // and assert the invariant that matters: trigger == due - lead.
    const saved = api.todos.find((t) => t.id === todo.id)!;
    const reminder = api.reminderForTodo(todo.id)!;
    expect(new Date(reminder.trigger_at).getTime()).toBe(
      new Date(saved.due_at!).getTime() - 1440 * 60_000
    );
    expect(new Date(saved.due_at!).getTime()).toBe(Math.floor(due.getTime() / 60_000) * 60_000);
    expect(reminder.room_id).toBe('!general:mbm.one');
  });

  test('clearing the reminder cancels it but keeps the due date', async ({ app, api }) => {
    const todo = api.seedTodo({
      text: 'Drop the reminder',
      due_at: new Date(Date.now() + 6 * 3_600_000).toISOString(),
      remind_before_minutes: 60,
    });
    api.seedReminder({ todo_id: todo.id });
    await gotoApp(app, 'Todos');

    await todoRow(app, 'Drop the reminder').getByTitle('Edit task').click();
    const form = todoEditForm(app);
    await chooseOption(selectField(form, 'Remind me'), 'No reminder');
    await form.getByRole('button', { name: 'Save' }).click();

    await expect(todoEditForm(app)).toHaveCount(0);
    await expect
      .poll(() => api.todos.find((t) => t.id === todo.id)?.remind_before_minutes)
      .toBeNull();
    expect(api.todos.find((t) => t.id === todo.id)?.due_at).toBeTruthy();
    expect(api.reminderForTodo(todo.id)).toBeUndefined();
  });

  test('clearing the due date cancels the reminder too', async ({ app, api }) => {
    const todo = api.seedTodo({
      text: 'Drop the date',
      due_at: new Date(Date.now() + 6 * 3_600_000).toISOString(),
      remind_before_minutes: 60,
    });
    api.seedReminder({ todo_id: todo.id });
    await gotoApp(app, 'Todos');

    await todoRow(app, 'Drop the date').getByTitle('Edit task').click();
    const form = todoEditForm(app);
    await clearDateTime(dateField(form, 'Due date'));
    await chooseOption(selectField(form, 'Remind me'), 'No reminder');
    await form.getByRole('button', { name: 'Save' }).click();

    await expect(todoEditForm(app)).toHaveCount(0);
    await expect.poll(() => api.todos.find((t) => t.id === todo.id)?.due_at).toBeNull();
    expect(api.reminderForTodo(todo.id)).toBeUndefined();
  });

  test('cancelling an edit discards the changes', async ({ app, api }) => {
    api.seedTodo({ text: 'Keep me as I am' });
    await gotoApp(app, 'Todos');

    await todoRow(app, 'Keep me as I am').getByTitle('Edit task').click();
    const form = todoEditForm(app);
    await form.locator('input[type="text"]').fill('Should not persist');
    await form.getByRole('button', { name: 'Cancel' }).click();

    await expect(todoEditForm(app)).toHaveCount(0);
    await expect(todoRow(app, 'Keep me as I am')).toBeVisible();
    expect(api.requests.some((r) => r.method === 'PATCH')).toBe(false);
  });

  test('rejects an empty task text on edit', async ({ app, api }) => {
    api.seedTodo({ text: 'Has text' });
    await gotoApp(app, 'Todos');

    await todoRow(app, 'Has text').getByTitle('Edit task').click();
    const form = todoEditForm(app);
    await form.locator('input[type="text"]').fill('   ');
    await form.getByRole('button', { name: 'Save' }).click();

    await expect(form.getByText('Task text cannot be empty.')).toBeVisible();
    expect(api.requests.some((r) => r.method === 'PATCH')).toBe(false);
  });

  test('shows a server error inside the row and keeps editing open', async ({ app, api }) => {
    api.seedTodo({ text: 'Server will refuse' });
    await gotoApp(app, 'Todos');

    await todoRow(app, 'Server will refuse').getByTitle('Edit task').click();
    const form = todoEditForm(app);
    api.failNext = { path: /^\/todos\/\d+$/, status: 400, error: 'Nope, not allowed' };
    await form.locator('input[type="text"]').fill('Renamed');
    await form.getByRole('button', { name: 'Save' }).click();

    await expect(form.getByText('Nope, not allowed')).toBeVisible();
    await expect(todoEditForm(app)).toHaveCount(1);
  });
});
