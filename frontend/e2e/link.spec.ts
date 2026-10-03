import { test, expect } from './fixtures';
import type { Page } from '@playwright/test';

/**
 * The page a `/link` message from a chat bot points at.
 *
 * The token arrives in the query string and must leave the address bar at once
 * (it is a bearer secret until it is claimed), so every spec also checks that.
 * The fake API picks the inspect answer from the token, which is how a spec
 * chooses the state it wants rendered.
 */

async function addVirtualAuthenticator(page: Page): Promise<void> {
  const client = await page.context().newCDPSession(page);
  await client.send('WebAuthn.enable');
  await client.send('WebAuthn.addVirtualAuthenticator', {
    options: {
      protocol: 'ctap2',
      transport: 'internal',
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: true,
      automaticPresenceSimulation: true,
    },
  });
}

test.describe('link page', () => {
  test('binds the chat account and scrubs the token from the address bar', async ({ app, api }) => {
    await app.goto('/link?token=bind-token');

    await expect(
      app.getByRole('heading', { name: /connect telegram account 42 to your workspace\?/i })
    ).toBeVisible();
    expect(new URL(app.url()).searchParams.has('token')).toBe(false);

    await app.getByRole('button', { name: 'Connect', exact: true }).click();

    await expect(app.getByText('Connected. You can close this tab.')).toBeVisible();
    const claim = api.requests.find((r) => r.url === '/link/claim');
    expect(claim?.body).toEqual({ token: 'bind-token' });
  });

  test('an expired token says to ask the bot again', async ({ app }) => {
    await app.goto('/link?token=expired-token');

    await expect(app.getByText('Send /link to the bot again.')).toBeVisible();
    await expect(app.getByRole('button', { name: 'Connect', exact: true })).toHaveCount(0);
  });

  test('a missing token reads as expired without calling the server', async ({ app, api }) => {
    await app.goto('/link');

    await expect(app.getByText('Send /link to the bot again.')).toBeVisible();
    expect(api.requests.some((r) => r.url === '/link/inspect')).toBe(false);
  });

  test('an allowlisted stranger can create an account with a passkey', async ({ anonApp, api }) => {
    await addVirtualAuthenticator(anonApp);
    await anonApp.goto('/link?token=signup-token');

    await expect(anonApp.getByRole('button', { name: 'Create an account with a passkey' })).toBeVisible();
    await expect(anonApp.getByRole('button', { name: 'I already have an account' })).toBeVisible();

    await anonApp.getByRole('button', { name: 'Create an account with a passkey' }).click();

    await expect(anonApp.getByText('Connected. You can close this tab.')).toBeVisible();
    const finish = api.requests.find((r) => r.url === '/link/signup/finish');
    expect(finish?.body).toMatchObject({ ceremony_id: 'ceremony_link_signup', token: 'signup-token' });
  });

  test('a signed-out owner is asked to sign in, then the token is inspected again', async ({ anonApp, api }) => {
    await addVirtualAuthenticator(anonApp);
    await anonApp.goto('/link?token=signin-token');

    await expect(anonApp.getByRole('button', { name: 'Sign in with a passkey' })).toBeVisible();
    // Not the global sign-in modal: this page owns the prompt.
    await expect(anonApp.getByRole('dialog')).toHaveCount(0);
  });

  test('a merge shows how many items move and needs a second confirmation', async ({ app, api }) => {
    await app.goto('/link?token=merge-token');

    await expect(app.getByText(/7 items/)).toBeVisible();
    await app.getByRole('button', { name: 'Merge and connect' }).click();

    await expect(app.getByText('Connected. You can close this tab.')).toBeVisible();
    expect(api.requests.find((r) => r.url === '/link/claim')?.body).toEqual({ token: 'merge-token' });
  });

  test('an account owned by someone else is explained, not offered', async ({ app }) => {
    await app.goto('/link?token=taken-token');

    await expect(app.getByText(/already connected to a different workspace/i)).toBeVisible();
    await expect(app.getByRole('button', { name: 'Connect', exact: true })).toHaveCount(0);
  });

  test('an account that is already yours says so', async ({ app }) => {
    await app.goto('/link?token=yours-token');

    await expect(app.getByText(/already connected to your workspace/i)).toBeVisible();
  });
});
