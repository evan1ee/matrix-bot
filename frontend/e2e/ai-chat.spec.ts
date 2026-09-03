import { test, expect, gotoApp, TAB } from './fixtures';

test('navigates to AI Assistant and renders suggestions', async ({ app }) => {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();
  await expect(app.getByRole('heading', { name: 'AI Assistant', exact: true })).toBeVisible();
  await expect(app.getByText('How can I help you today?')).toBeVisible();
  await expect(app.getByText('What tasks do I have due today?')).toBeVisible();
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

test('configures custom AI model and endpoint in settings', async ({ app }) => {
  await gotoApp(app);
  await app.getByRole('button', { name: TAB.ai, exact: true }).click();

  await app.locator('button[title="Configure AI model & API keys"]').click();
  await expect(app.getByText('AI Assistant Settings')).toBeVisible();

  await app.locator('input[placeholder*="llama3.2"]').fill('gpt-4o-mini');
  await app.getByRole('button', { name: 'Save Settings' }).click();

  await expect(app.getByText('AI Assistant Settings')).not.toBeVisible();
  await expect(app.getByText('gpt-4o-mini')).toBeVisible();
});
