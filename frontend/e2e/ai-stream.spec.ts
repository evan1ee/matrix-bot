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
