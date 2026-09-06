import { test, expect, gotoApp, chooseOption, selectField, TAB } from './fixtures';

test('navigates to AI Assistant and renders suggestions', async ({ app }) => {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
  await expect(app.getByRole('heading', { name: 'AI Assistant', exact: true })).toBeVisible();
  await expect(app.getByText('How can I help you today?')).toBeVisible();
  await expect(app.getByText('What tasks do I have due today?')).toBeVisible();
});

test('clicking a recommended prompt sends immediately and clears input', async ({ app }) => {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();

  const promptText = 'What tasks do I have due today?';
  const suggestionBtn = app.getByRole('button', { name: promptText });
  await expect(suggestionBtn).toBeVisible();

  await suggestionBtn.click();

  // Input textarea should be empty immediately
  const input = app.getByPlaceholder(/Ask AI assistant/i);
  await expect(input).toHaveValue('');

  // User message should appear immediately in chat
  await expect(app.getByText(promptText, { exact: true })).toBeVisible();

  // Assistant reply should arrive
  await expect(app.getByText(/You have \d+ task\(s\) currently recorded in your list\./i)).toBeVisible();

  const brainDir = process.env.BRAIN_DIR || '/Users/nosinx/.gemini/antigravity/brain/d97c3792-b421-4577-aa66-bc70c73a4412';
  if (brainDir) {
    await app.screenshot({ path: `${brainDir}/ai_chat_recommendation_sent.png`, fullPage: false });
  }
});

test('renders markdown table, headers, and filters empty assistant replies', async ({ app }) => {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();

  const promptText = 'Show a breakdown of my portfolio and cash balance';
  const suggestionBtn = app.getByRole('button', { name: promptText });
  await expect(suggestionBtn).toBeVisible();

  await suggestionBtn.click();

  // Tool result should appear
  await expect(app.getByText('Tool Result:')).toBeVisible();
  await expect(app.getByText('get_my_portfolio_summary')).toBeVisible();

  // Markdown table should be rendered as HTML table elements, not raw markdown pipes
  const table = app.locator('table');
  await expect(table).toBeVisible();
  await expect(table.locator('th', { hasText: 'Asset' })).toBeVisible();
  await expect(table.locator('td', { hasText: '$7,998.16' })).toBeVisible();

  // Exactly 2 markdown bubbles should exist: 1 for user prompt, 1 for assistant reply (no empty assistant bubble)
  const markdownBubbles = app.locator('.markdown-body');
  await expect(markdownBubbles).toHaveCount(2);
  await expect(markdownBubbles.nth(0)).toHaveText(promptText);
  await expect(markdownBubbles.nth(1)).not.toBeEmpty();

  const brainDir = process.env.BRAIN_DIR || '/Users/nosinx/.gemini/antigravity/brain/d97c3792-b421-4577-aa66-bc70c73a4412';
  if (brainDir) {
    await app.screenshot({ path: `${brainDir}/ai_chat_markdown_rendered.png`, fullPage: false });
  }
});

test('shows optimistic user message and clears input while AI is replying', async ({ app }) => {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();

  let unblock: () => void = () => {};
  const gate = new Promise<void>((resolve) => {
    unblock = resolve;
  });

  await app.route('**/api/v1/ai/chat', async (route) => {
    await gate;
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        conversation_id: 'conv_delay_test',
        message: 'Reply after thinking finished!',
        model_used: 'gpt-4o-mini',
        tool_calls_executed: [],
      }),
    });
  });

  await app.route('**/api/v1/ai/conversations/conv_delay_test/messages', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([
        {
          id: 'msg_u',
          conversation_id: 'conv_delay_test',
          role: 'user',
          content: 'What tasks do I have due today?',
          created_at: new Date().toISOString(),
        },
        {
          id: 'msg_a',
          conversation_id: 'conv_delay_test',
          role: 'assistant',
          content: 'Reply after thinking finished!',
          created_at: new Date().toISOString(),
        },
      ]),
    });
  });

  await app.route('**/api/v1/ai/conversations', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([
        {
          id: 'conv_delay_test',
          owner_id: 'owner',
          room_id: 'web_chat',
          created_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
        },
      ]),
    });
  });

  const promptText = 'What tasks do I have due today?';
  const suggestionBtn = app.getByRole('button', { name: promptText });
  await suggestionBtn.click();

  // Input textarea should be empty immediately!
  const input = app.getByPlaceholder(/Ask AI assistant/i);
  await expect(input).toHaveValue('');

  // Optimistic user message bubble should appear immediately
  await expect(app.getByText(promptText, { exact: true })).toBeVisible();

  // Thinking indicator should be visible
  await expect(app.getByText('Thinking and executing tools...')).toBeVisible();

  const brainDir = process.env.BRAIN_DIR || '/Users/nosinx/.gemini/antigravity/brain/d97c3792-b421-4577-aa66-bc70c73a4412';
  if (brainDir) {
    await app.screenshot({ path: `${brainDir}/ai_chat_optimistic_thinking.png`, fullPage: false });
  }

  unblock();
  await expect(app.getByText('Reply after thinking finished!')).toBeVisible();
});

test('sends a chat prompt and receives AI assistant reply', async ({ app }) => {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();

  await app.getByPlaceholder(/Ask AI assistant/i).fill('Explain Rust lifetimes simply');
  await app.locator('form button[type="submit"]').click();

  await expect(app.getByText('Explain Rust lifetimes simply', { exact: true })).toBeVisible();
  await expect(app.getByText(/I received your prompt: "Explain Rust lifetimes simply"/i)).toBeVisible();
});

test('triggers tool-augmented query to inspect tasks', async ({ app, api }) => {
  api.seedTodo({ text: 'Deploy Matrix Bot to staging' });
  api.seedTodo({ text: 'Audit cryptographic recovery keys' });

  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();

  await app.getByPlaceholder(/Ask AI assistant/i).fill('What tasks do I have?');
  await app.locator('form button[type="submit"]').click();

  await expect(app.getByText(/Tool Result:/i)).toBeVisible();
  await expect(app.getByText('get_my_tasks')).toBeVisible();
  await expect(app.getByText('You have 2 task(s) currently recorded in your list.')).toBeVisible();
});

test('resets conversation via New Chat button', async ({ app }) => {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();

  await app.getByPlaceholder(/Ask AI assistant/i).fill('Hello AI');
  await app.locator('form button[type="submit"]').click();
  await expect(app.getByText('Hello AI', { exact: true })).toBeVisible();

  await app.getByRole('button', { name: /New Chat/i }).click();
  await expect(app.getByText('How can I help you today?')).toBeVisible();
  await expect(app.getByText('Hello AI')).not.toBeVisible();
});

test('configures AI model and endpoint via dropdowns in settings', async ({ app }) => {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();

  await app.locator('button[title="Configure AI model & API keys"]').click();
  await expect(app.getByText('AI Assistant Settings')).toBeVisible();

  // Switch provider to DeepSeek via dropdown
  await chooseOption(selectField(app, 'API Endpoint / Provider'), 'DeepSeek');

  await app.getByRole('button', { name: 'Save Settings' }).click();

  await expect(app.getByText('AI Assistant Settings')).not.toBeVisible();
  await expect(app.getByText('deepseek-chat')).toBeVisible();

  // Re-open and switch provider to xAI / Grok
  await app.locator('button[title="Configure AI model & API keys"]').click();
  await expect(app.getByText('AI Assistant Settings')).toBeVisible();
  await chooseOption(selectField(app, 'API Endpoint / Provider'), 'xAI / Grok');

  // Test API Key verification
  await app.locator('input[type="password"]').fill('xai-test-valid-key');
  await app.getByRole('button', { name: /verify key/i }).click();
  await expect(app.getByText('Connection successful and API key verified!')).toBeVisible();

  // Test invalid key verification feedback
  await app.locator('input[type="password"]').fill('invalid_key');
  await app.getByRole('button', { name: /verify key/i }).click();
  await expect(app.getByText(/Invalid API key/i)).toBeVisible();

  // Restore valid key and save
  await app.locator('input[type="password"]').fill('xai-test-valid-key');
  await app.getByRole('button', { name: 'Save Settings' }).click();
  await expect(app.getByText('AI Assistant Settings')).not.toBeVisible();
  await expect(app.getByText('grok-4.6')).toBeVisible();
});
