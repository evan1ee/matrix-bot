import { test, expect, gotoApp, TAB } from './fixtures';
import type { FakeAiMessage, FakeApi } from './fake-api';

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
  await expect(app.getByPlaceholder(/Ask AI assistant/i)).toBeFocused();
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
  // Nobody pressed Stop: the turn may still be finishing on the server.
  await expect(app.getByText('Waiting for the result…')).toBeVisible();
  await expect(app.getByText('Stopping…')).toHaveCount(0);

  api.aiMessages.push(stored);
  api.pushEvent('AiConversationUpdated', { id: CONV, owner_id: 'owner' });
  await expect(app.getByText('Stored after the drop.')).toBeVisible({ timeout: 20_000 });
  await expect(app.getByText('Waiting for the result…')).toHaveCount(0);
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
  // Paragraphs only: the thread switcher also shows the auto title "hi".
  await expect(app.locator('p', { hasText: /^hi$/ })).toHaveCount(2);
});

test('a retraction of the open segment keeps the earlier text and drops the partial', async ({ app, api }) => {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
  await api.scriptStream({
    steps: [
      { event: started },
      { event: { type: 'delta', text: 'Before tool' } },
      { event: { type: 'segment_end' } },
      { event: toolStarted('add_todo') },
      { event: toolFinished('add_todo', 'ok') },
      { event: { type: 'delta', text: 'partial' } },
      { event: { type: 'segment_retracted' } },
      { event: { type: 'delta', text: 'full answer' } },
      { event: { type: 'segment_end' } },
    ],
    hold: true,
  });

  await send(app, 'Retract mid-stream');
  const live = app.getByTestId('live-turn');
  await expect(live).toContainText('full answer');
  await expect(live).toContainText('Before tool');
  await expect(live).not.toContainText('partial');
});

/** A row the server stores while the turn runs, as `run_turn` does. */
function storeRow(api: FakeApi, conversationId: string, id: string, role: FakeAiMessage['role'], content: string, extra: Partial<FakeAiMessage> = {}) {
  api.aiMessages.push({
    id,
    conversation_id: conversationId,
    role,
    content,
    tool_call_id: null,
    tool_name: null,
    created_at: new Date().toISOString(),
    ...extra,
  });
}

const doneIn = (conversationId: string, message: string) => ({ ...done(message), conversation_id: conversationId });

const historyOf = (app: import('@playwright/test').Page, conversationId: string) =>
  app.waitForResponse((r) => r.url().includes(`/ai/conversations/${conversationId}/messages`), { timeout: 20_000 });

/** Lets a fetched history render before counting: a count can only be wrong after it has. */
async function expectOnce(app: import('@playwright/test').Page, text: string) {
  await app.waitForTimeout(500);
  await expect(app.getByText(text, { exact: true })).toHaveCount(1);
}

test('a new conversation shows the prompt once while the server stores the turn', async ({ app, api }) => {
  const LIVE = 'conv_live';
  const prompt = 'Remember the milk';
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
  const stream = await api.scriptStream({ hold: true });

  await send(app, prompt);
  // The server creates the conversation and stores the prompt right after `started`.
  const now = new Date().toISOString();
  api.aiConversations.push({ id: LIVE, owner_id: 'owner', room_id: '!room1:example.com', created_at: now, updated_at: now });
  storeRow(api, LIVE, 'm_live_u', 'user', prompt);
  const fetched = historyOf(app, LIVE);
  await stream.send({ type: 'started', conversation_id: LIVE });
  await fetched;
  await expectOnce(app, prompt);

  // A tool round is stored after it runs, and something refetches the history mid-turn.
  await stream.send(toolStarted('add_todo'));
  storeRow(api, LIVE, 'm_live_t', 'tool', '{"ok":true}', { tool_call_id: 'call_0', tool_name: 'add_todo', tool_outcome: 'ok' });
  await stream.send(toolFinished('add_todo', 'ok'));
  const refetched = historyOf(app, LIVE);
  api.pushEvent('AiConversationUpdated', { id: LIVE, owner_id: 'owner' });
  await refetched;
  await expectOnce(app, prompt);
  await expect(app.getByTestId('tool-step')).toHaveCount(1);

  await stream.send({ type: 'delta', text: 'Added it.' });
  await stream.send({ type: 'segment_end' });
  await expect(app.getByTestId('live-turn')).toContainText('Added it.');
  storeRow(api, LIVE, 'm_live_a', 'assistant', 'Added it.');
  api.aiMessagesDelayMs = 500;
  // Records whether the reply ever leaves the page once it was there.
  await app.evaluate(() => {
    const w = window as unknown as { __replyGap: boolean };
    w.__replyGap = false;
    let seen = false;
    const tick = () => {
      const shown = document.body.innerText.includes('Added it.');
      if (shown) seen = true;
      else if (seen) w.__replyGap = true;
      requestAnimationFrame(tick);
    };
    tick();
  });
  await stream.send(doneIn(LIVE, 'Added it.'));
  await stream.end();

  await expect(app.getByTestId('live-turn')).toHaveCount(0);
  await expectOnce(app, prompt);
  await expect(app.getByText('Added it.', { exact: true })).toHaveCount(1);
  await expect(app.getByTestId('tool-step')).toHaveCount(1);
  expect(await app.evaluate(() => (window as unknown as { __replyGap: boolean }).__replyGap)).toBe(false);
});

test('returning to the page mid-turn shows the prompt and steps once', async ({ app, api }) => {
  seedStoredTurn(api, 'Earlier question', 'Earlier answer.');
  const prompt = 'Tidy my list';
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
  await expect(app.getByText('Earlier answer.')).toBeVisible();
  const stream = await api.scriptStream({ hold: true });

  await send(app, prompt);
  storeRow(api, CONV, 'm_next_u', 'user', prompt);
  await stream.send(started);
  await stream.send(toolStarted('add_todo'));
  storeRow(api, CONV, 'm_next_t', 'tool', '{"ok":true}', { tool_call_id: 'call_0', tool_name: 'add_todo', tool_outcome: 'ok' });
  await stream.send(toolFinished('add_todo', 'ok'));
  await expect(app.getByTestId('tool-step')).toHaveAttribute('data-outcome', 'ok');

  // Away long enough for the history to go stale, so coming back refetches it.
  await app.getByRole('button', { name: TAB.todo, exact: true }).click();
  await app.waitForTimeout(5_500);
  const refetched = historyOf(app, CONV);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
  await refetched;
  await expectOnce(app, prompt);
  await expectOnce(app, 'Earlier question');
  await expect(app.getByTestId('tool-step')).toHaveCount(1);
  await expect(app.getByTestId('live-turn')).toBeVisible();

  storeRow(api, CONV, 'm_next_a', 'assistant', 'Tidied.');
  await stream.send({ type: 'delta', text: 'Tidied.' });
  await stream.send({ type: 'segment_end' });
  await stream.send(done('Tidied.'));
  await stream.end();
  await expect(app.getByTestId('live-turn')).toHaveCount(0);
  await expectOnce(app, prompt);
  await expect(app.getByText('Tidied.', { exact: true })).toHaveCount(1);
});

test('stop before started frees the composer at once', async ({ app, api }) => {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
  await api.scriptStream({ hold: true });

  await send(app, 'Never mind');
  await app.getByRole('button', { name: 'Stop' }).click();
  const input = app.getByPlaceholder(/Ask AI assistant/i);
  await input.fill('Next question');
  await expect(app.locator('form button[type="submit"]')).toBeEnabled();
  await expect(app.getByText('Stopping…')).toHaveCount(0);
  expect(await api.streamAborts()).toBe(1);
});

test('an error leaves no tool step spinning', async ({ app, api }) => {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
  await api.scriptStream({
    steps: [{ event: started }, { event: toolStarted('add_todo') }, { event: { type: 'error', message: 'The AI turn timed out' } }],
    hold: true,
  });

  await send(app, 'Slow tool');
  await expect(app.getByText(/Generation failed/)).toBeVisible();
  const step = app.getByTestId('tool-step');
  await expect(step).toHaveAttribute('data-outcome', 'none');
  await expect(step).toHaveText('Add todo');
});

test('a malformed frame stops reading the stream', async ({ app, api }) => {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
  await api.scriptStream({ steps: [{ event: started }, { raw: 'data: {not json\n\n' }], hold: true });

  await send(app, 'Garbled');
  await expect.poll(() => api.streamCancels()).toBe(1);
});
