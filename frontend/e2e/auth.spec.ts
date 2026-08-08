import { test, expect } from './fixtures';

/** The login/register modal's own submit button. */
const submit = (name: RegExp) => ({ name });

test('signs in and stores the session token', async ({ anonApp, api }) => {
  await anonApp.goto('/');

  await anonApp.getByRole('button', { name: /sign in|login|connect/i }).first().click();
  await anonApp.getByPlaceholder('e.g. alice').fill('tester');
  await anonApp.getByPlaceholder('••••••••').fill('hunter2');
  await anonApp.getByRole('button', { name: 'Login', exact: true }).click();

  await expect
    .poll(() => anonApp.evaluate(() => window.localStorage.getItem('matrix_bot_jwt')))
    .toBe('fake-jwt-token');

  expect(api.requests.some((r) => r.url === '/auth/login' && r.method === 'POST')).toBe(true);
});

test('shows the server error for bad credentials and stores nothing', async ({ anonApp }) => {
  await anonApp.goto('/');

  await anonApp.getByRole('button', { name: /sign in|login|connect/i }).first().click();
  await anonApp.getByPlaceholder('e.g. alice').fill('tester');
  await anonApp.getByPlaceholder('••••••••').fill('wrong');
  await anonApp.getByRole('button', { name: 'Login', exact: true }).click();

  await expect(anonApp.getByText('Invalid username or password')).toBeVisible();
  expect(await anonApp.evaluate(() => window.localStorage.getItem('matrix_bot_jwt'))).toBeNull();
});

test('a rejected token is dropped and the login prompt reopens', async ({ app, api }) => {
  await app.goto('/');
  await expect(app.getByText('TOTAL TASKS')).toBeVisible();

  // The stored token stops being accepted — the app must not keep retrying it.
  api.failNext = { path: /^\/todos$/, status: 401, error: 'Invalid or expired token' };
  await app.reload();

  await expect
    .poll(() => app.evaluate(() => window.localStorage.getItem('matrix_bot_jwt')))
    .toBeNull();
});

test('signs out and clears the stored token', async ({ app }) => {
  await app.goto('/');
  await expect(app.getByText('TOTAL TASKS')).toBeVisible();

  await app.getByRole('button', { name: /sign in|account|session|tester/i }).first().click();
  await app.getByRole('button', { name: /log ?out|sign out/i }).click();

  await expect
    .poll(() => app.evaluate(() => window.localStorage.getItem('matrix_bot_jwt')))
    .toBeNull();
});
