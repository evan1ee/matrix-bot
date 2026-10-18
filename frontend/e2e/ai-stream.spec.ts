import { test, expect, gotoApp, TAB } from './fixtures';
import type { FakeApi } from './fake-api';

const CONV = 'conv_stream';

/** A conversation whose stored history already holds the turn the script streams. */
function seedStoredTurn(api: FakeApi, prompt: string, reply: string) {
  const now = new Date().toISOString();
  api.aiConversations.push({ id: CONV, owner_id: 'owner', room_id: '!room1:example.com', created_at: now, updated_at: now });
  api.aiMessages.push(
    { id: 'm_u', conversation_id: CONV, role: 'user', content: prompt, tool_call_id: null, tool_name: null, created_at: now },
    { id: 'm_a', conversation_id: CONV, role: 'assistant', content: reply, tool_call_id: null, tool_name: null, created_at: now }
  );
}

const started = { type: 'started', conversation_id: CONV };
const done = (message: string) => ({
  type: 'done',
  conversation_id: CONV,
  message,
  model_used: 'gpt-4o-mini',
  tool_calls_executed: 0,
});

async function send(app: import('@playwright/test').Page, text: string) {
  await app.getByPlaceholder(/Ask AI assistant/i).fill(text);
  await app.locator('form button[type="submit"]').click();
}

test('text arrives in pieces', async ({ app, api }) => {
  seedStoredTurn(api, 'Tell me in two parts', 'First piece. Second piece.');
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
  await api.scriptStream({
    steps: [
      { event: started },
      { event: { type: 'delta', text: 'First piece.' } },
      { event: { type: 'delta', text: ' Second piece.' }, delayMs: 400 },
      { event: { type: 'segment_end' } },
      // The stored history takes over after `done`, so leave a window to assert the live text.
      { event: done('First piece. Second piece.'), delayMs: 1500 },
    ],
  });

  await send(app, 'Tell me in two parts');
  const live = app.getByTestId('live-turn');
  await expect(live).toHaveText('First piece.');
  await expect(live).toHaveText('First piece. Second piece.');
});

test('multibyte text arriving across chunks renders intact', async ({ app, api }) => {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
  await api.scriptStream({
    steps: [
      { event: started },
      // The frame is `data: {"type":"delta","text":"你好🎉"}`; byte 31 falls inside 你.
      { event: { type: 'delta', text: '你好🎉' }, splitAtByte: 31 },
    ],
    hold: true,
  });

  await send(app, 'Say hello');
  await expect(app.getByTestId('live-turn')).toHaveText('你好🎉');
});

test('the turn keeps running after navigating away', async ({ app, api }) => {
  seedStoredTurn(api, 'Keep going', 'The finished reply.');
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
  const stream = await api.scriptStream({
    steps: [{ event: started }, { event: { type: 'delta', text: 'Still working' } }],
    hold: true,
  });

  await send(app, 'Keep going');
  await expect(app.getByTestId('live-turn')).toHaveText('Still working');

  await app.getByRole('button', { name: TAB.todo, exact: true }).click();
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
  await expect(app.getByTestId('live-turn')).toHaveText('Still working');
  expect(await api.streamAborts()).toBe(0);

  await stream.send({ type: 'delta', text: ' and done.' });
  await stream.send({ type: 'segment_end' });
  await stream.send(done('The finished reply.'));
  await stream.end();
  await expect(app.getByText('The finished reply.')).toBeVisible();
  await expect(app.getByTestId('live-turn')).toHaveCount(0);
});

test('a non-2xx start shows the server error and restores the prompt', async ({ app, api }) => {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
  await api.scriptStream({ status: 429, errorBody: { error: 'Too many AI turns are running', code: 429 } });

  await send(app, 'This will be refused');
  await expect(app.getByText('Too many AI turns are running')).toBeVisible();
  await expect(app.getByPlaceholder(/Ask AI assistant/i)).toHaveValue('This will be refused');
});

test('a stream that ends without done waits for the conversation update, then shows history', async ({ app, api }) => {
  seedStoredTurn(api, 'Cut short', 'Stored after the cut.');
  const stored = api.aiMessages.pop()!; // the server stores it only after the cut
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
  const stream = await api.scriptStream({
    steps: [{ event: started }, { event: { type: 'delta', text: 'Partial' } }],
    hold: true,
  });

  await send(app, 'Cut short');
  await expect(app.getByTestId('live-turn')).toHaveText('Partial');
  await stream.end();
  // Still waiting: the server has not said the turn was stored.
  await expect(app.getByTestId('live-turn')).toHaveText('Partial');

  api.aiMessages.push(stored);
  api.pushEvent('AiConversationUpdated', { id: CONV, owner_id: 'owner' });
  await expect(app.getByText('Stored after the cut.')).toBeVisible({ timeout: 20_000 });
  await expect(app.getByTestId('live-turn')).toHaveCount(0);
});

test('a network failure before started shows an error and frees the input', async ({ app, api }) => {
  seedStoredTurn(api, 'Earlier question', 'Earlier answer.');
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
  await expect(app.getByText('Earlier answer.')).toBeVisible();
  await api.scriptStream({ networkError: true });

  await send(app, 'Follow-up while offline');
  await expect(app.getByText(/Generation failed/)).toBeVisible();
  const input = app.getByPlaceholder(/Ask AI assistant/i);
  await expect(input).toHaveValue('Follow-up while offline');
  await expect(app.locator('form button[type="submit"]')).toBeEnabled();
});

const toolStarted = (name: string, callId = 'call_0') => ({ type: 'tool_started', call_id: callId, name });
const toolFinished = (name: string, outcome: string, callId = 'call_0') => ({
  type: 'tool_finished',
  call_id: callId,
  name,
  outcome,
});

test('a tool step goes from running to done', async ({ app, api }) => {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
  const stream = await api.scriptStream({ steps: [{ event: started }, { event: toolStarted('add_todo') }], hold: true });

  await send(app, 'Add a todo');
  const step = app.getByTestId('tool-step');
  await expect(step).toHaveText('Adding todo…');
  await expect(step).toHaveAttribute('data-outcome', 'running');

  await stream.send(toolFinished('add_todo', 'ok'));
  await expect(step).toHaveText('Add todo');
  await expect(step).toHaveAttribute('data-outcome', 'ok');
});

test('a refused tool shows needs confirmation', async ({ app, api }) => {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
  await api.scriptStream({
    steps: [{ event: started }, { event: toolStarted('delete_todo') }, { event: toolFinished('delete_todo', 'needs_confirmation') }],
    hold: true,
  });

  await send(app, 'Delete my todo');
  const step = app.getByTestId('tool-step');
  await expect(step).toHaveText('Delete todo needs confirmation');
  await expect(step).toHaveAttribute('data-outcome', 'needs_confirmation');
});

test('stop shows stopping and then the stored stopped reply', async ({ app, api }) => {
  seedStoredTurn(api, 'Long task', 'Half an answer _(stopped)_');
  const stored = api.aiMessages.splice(1, 1)[0]; // stored by the server only after the stop
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
  await api.scriptStream({
    steps: [{ event: started }, { event: { type: 'delta', text: 'Half an answer' } }],
    hold: true,
  });

  await send(app, 'Long task');
  await expect(app.getByTestId('live-turn')).toHaveText('Half an answer');
  await app.getByRole('button', { name: 'Stop' }).click();
  await expect(app.getByText('Stopping…')).toBeVisible();
  expect(await api.streamAborts()).toBe(1);

  api.aiMessages.push(stored);
  api.pushEvent('AiConversationUpdated', { id: CONV, owner_id: 'owner' });
  await expect(app.getByText('(stopped)')).toBeVisible({ timeout: 20_000 });
  await expect(app.getByTestId('live-turn')).toHaveCount(0);
  await expect(app.getByText('Stopping…')).toHaveCount(0);
});

test('escape stops the turn', async ({ app, api }) => {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
  await api.scriptStream({ steps: [{ event: started }], hold: true });

  await send(app, 'Stop me with the keyboard');
  await expect(app.getByRole('button', { name: 'Stop' })).toBeVisible();
  const input = app.getByPlaceholder(/Ask AI assistant/i);
  await input.focus();
  await input.press('Escape');
  await expect(app.getByText('Stopping…')).toBeVisible();
  expect(await api.streamAborts()).toBe(1);
});

test('a dropped stream falls back to stored history', async ({ app, api }) => {
  seedStoredTurn(api, 'Dropped', 'Stored after the drop.');
  const stored = api.aiMessages.pop()!;
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
  await api.scriptStream({ steps: [{ event: started }] }); // closes without done or error

  await send(app, 'Dropped');
  await expect(app.getByText('Stopping…')).toBeVisible();

  api.aiMessages.push(stored);
  api.pushEvent('AiConversationUpdated', { id: CONV, owner_id: 'owner' });
  await expect(app.getByText('Stored after the drop.')).toBeVisible({ timeout: 20_000 });
  await expect(app.getByText('Stopping…')).toHaveCount(0);
});

test('stored tool rows render as collapsed steps', async ({ app, api }) => {
  const now = new Date().toISOString();
  api.aiConversations.push({ id: CONV, owner_id: 'owner', room_id: '!room1:example.com', created_at: now, updated_at: now });
  api.aiMessages.push(
    { id: 'm1', conversation_id: CONV, role: 'user', content: 'Do things', tool_call_id: null, tool_name: null, created_at: now },
    { id: 'm2', conversation_id: CONV, role: 'tool', content: '{"error":"boom"}', tool_call_id: 'c1', tool_name: 'add_todo', tool_outcome: 'failed', created_at: now },
    { id: 'm3', conversation_id: CONV, role: 'tool', content: '{"legacy":true}', tool_call_id: 'c2', tool_name: 'get_my_tasks', tool_outcome: null, created_at: now },
    { id: 'm4', conversation_id: CONV, role: 'assistant', content: 'All done.', tool_call_id: null, tool_name: null, created_at: now }
  );
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();

  const steps = app.getByTestId('tool-step');
  await expect(steps).toHaveCount(2);
  await expect(steps.nth(0)).toContainText('Add todo failed');
  await expect(steps.nth(0)).toHaveAttribute('data-outcome', 'failed');
  await expect(steps.nth(1)).toContainText('Look up tasks');
  await expect(steps.nth(1)).toHaveAttribute('data-outcome', 'none');
  await expect(app.getByText('{"error":"boom"}')).toBeHidden();

  await steps.nth(0).locator('summary').click();
  await expect(app.getByText('{"error":"boom"}')).toBeVisible();
  await expect(app.getByText('{"legacy":true}')).toBeHidden();
  await steps.nth(1).locator('summary').click();
  await expect(app.getByText('{"legacy":true}')).toBeVisible();
});

test('a failed turn does not come back after leaving the page', async ({ app, api }) => {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
  await api.scriptStream({ networkError: true });

  await send(app, 'Offline question');
  await expect(app.getByText(/Generation failed/)).toBeVisible();
  await app.getByPlaceholder(/Ask AI assistant/i).fill('');

  await app.getByRole('button', { name: TAB.todo, exact: true }).click();
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
  await expect(app.getByText('How can I help you today?')).toBeVisible();
  await expect(app.getByText(/Generation failed/)).toHaveCount(0);
  await expect(app.getByPlaceholder(/Ask AI assistant/i)).toHaveValue('');
});

test('the same prompt can be sent twice', async ({ app }) => {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();

  await send(app, 'hi');
  await expect(app.getByText(/I received your prompt/).first()).toBeVisible();
  await expect(app.getByTestId('live-turn')).toHaveCount(0);
  await send(app, 'hi');
  await expect(app.getByText(/I received your prompt/)).toHaveCount(2);
  await expect(app.getByText('hi', { exact: true })).toHaveCount(2);
});
