import { test, expect, gotoApp, TAB } from './fixtures';
import type { FakeApi } from './fake-api';

const NOW = new Date().toISOString();

function seedThread(api: FakeApi, id: string, title: string | null, prompt: string, reply: string) {
  api.aiConversations.push({
    id,
    owner_id: 'owner',
    room_id: '!room1:example.com',
    title,
    created_at: NOW,
    updated_at: NOW,
  });
  api.aiMessages.push(
    { id: `${id}_u`, conversation_id: id, role: 'user', content: prompt, tool_call_id: null, tool_name: null, created_at: NOW },
    { id: `${id}_a`, conversation_id: id, role: 'assistant', content: reply, tool_call_id: null, tool_name: null, created_at: NOW }
  );
}

async function openAi(app: import('@playwright/test').Page) {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
}

async function send(app: import('@playwright/test').Page, text: string) {
  await app.getByPlaceholder(/Ask AI assistant/i).fill(text);
  await app.locator('form button[type="submit"]').click();
}

test('new chat keeps the old thread and switching back restores its history', async ({ app, api }) => {
  seedThread(api, 'conv_a', 'Weekly plan', 'Plan my week', 'Here is the plan.');
  await openAi(app);
  await expect(app.getByText('Here is the plan.')).toBeVisible();

  await app.getByRole('button', { name: /New Chat/i }).click();
  await expect(app.getByText('How can I help you today?')).toBeVisible();
  await expect(app.getByText('Here is the plan.')).not.toBeVisible();
  expect(api.aiConversations.map((c) => c.id)).toContain('conv_a');

  await app.getByRole('button', { name: 'Switch conversation' }).click();
  await app.getByRole('option', { name: /Weekly plan/ }).click();
  await expect(app.getByText('Here is the plan.')).toBeVisible();
});

test('the first reply titles the thread and the title can be renamed', async ({ app, api }) => {
  await openAi(app);
  await send(app, 'Plan my week');
  await expect(app.getByRole('button', { name: 'Switch conversation' })).toContainText('Plan my week');

  await app.getByRole('button', { name: 'Rename conversation' }).click();
  const input = app.getByLabel('Conversation title');
  await input.fill('Q4 planning');
  await input.press('Enter');
  await expect(app.getByRole('button', { name: 'Switch conversation' })).toContainText('Q4 planning');
  expect(api.aiConversations[0].title).toBe('Q4 planning');
});

test('the model badge shows the configured model', async ({ app, api }) => {
  api.aiConfig.model_name = 'claude-sonnet-5-5';
  await openAi(app);
  await expect(app.getByText('claude-sonnet-5-5', { exact: true })).toBeVisible();
});

test('copy puts the message markdown on the clipboard', async ({ app, api, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  seedThread(api, 'conv_a', 'Notes', 'Say hi', '**Hello** there');
  await openAi(app);
  await app.getByRole('button', { name: 'Copy markdown' }).last().click();
  await expect(app.getByText('Copied')).toBeVisible();
  expect(await app.evaluate(() => navigator.clipboard.readText())).toBe('**Hello** there');
});

test('edit and resend truncates from the edited message', async ({ app, api }) => {
  seedThread(api, 'conv_a', 'Notes', 'First question', 'First answer');
  await openAi(app);
  await expect(app.getByText('First answer')).toBeVisible();

  await app.getByRole('button', { name: 'Edit and resend' }).click();
  const composer = app.getByPlaceholder(/Ask AI assistant/i);
  await expect(composer).toHaveValue('First question');
  await expect(app.getByText(/replaced when you send/)).toBeVisible();

  await composer.fill('Better question');
  await app.locator('form button[type="submit"]').click();

  await expect(app.getByText('First answer')).not.toBeVisible();
  await expect(app.getByText('First question')).not.toBeVisible();
  await expect(app.getByText('Better question', { exact: true })).toBeVisible();
  expect(
    api.requests.some((r) => r.method === 'POST' && r.url.includes('/ai/conversations/conv_a/truncate'))
  ).toBe(true);
});

test('cancelling an edit leaves the thread untouched', async ({ app, api }) => {
  seedThread(api, 'conv_a', 'Notes', 'First question', 'First answer');
  await openAi(app);
  await app.getByRole('button', { name: 'Edit and resend' }).click();
  await app.getByRole('button', { name: 'Cancel edit' }).click();
  await expect(app.getByPlaceholder(/Ask AI assistant/i)).toHaveValue('');
  await expect(app.getByText('First answer')).toBeVisible();
  expect(api.requests.some((r) => r.url.includes('/truncate'))).toBe(false);
});
