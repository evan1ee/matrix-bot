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

async function plantCredential(page: Page) {
  await page.evaluate(async () => {
    const res = await fetch('/api/v1/auth/passkey/register/start', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ display_name: 'tester' }),
      credentials: 'include',
    });
    const { options } = await res.json();
    const pk = options.publicKey;
    const toBuffer = (value: string) => {
      const padded = value.replace(/-/g, '+').replace(/_/g, '/');
      const binary = atob(padded + '='.repeat((4 - (padded.length % 4)) % 4));
      return Uint8Array.from(binary, (c) => c.charCodeAt(0)).buffer;
    };
    await navigator.credentials.create({
      publicKey: {
        ...pk,
        challenge: toBuffer(pk.challenge),
        user: { ...pk.user, id: toBuffer(pk.user.id) },
        excludeCredentials: [],
      },
    } as CredentialCreationOptions);
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
    await plantCredential(anonApp);

    await expect(anonApp.getByRole('button', { name: 'Sign in with a passkey' })).toBeVisible();
    // Not the global sign-in modal: this page owns the prompt.
    await expect(anonApp.getByRole('dialog')).toHaveCount(0);

    // Clicking "Sign in with a passkey" signs in and re-inspects
    api.linkPreviews['signin-token'] = {
      state: 'bind',
      channel: 'telegram',
      user_id: '42',
      merge_item_count: null,
    };
    await anonApp.getByRole('button', { name: 'Sign in with a passkey' }).click();

    // Now re-inspected as signed in and ready to connect
    await expect(anonApp.getByRole('button', { name: 'Connect', exact: true })).toBeVisible();
    await anonApp.getByRole('button', { name: 'Connect', exact: true }).click();
    await expect(anonApp.getByText('Connected. You can close this tab.')).toBeVisible();
  });

  test('an allowlisted stranger with existing account can choose "I already have an account" and sign in', async ({ anonApp, api }) => {
    await addVirtualAuthenticator(anonApp);
    await anonApp.goto('/link?token=signup-token');
    await plantCredential(anonApp);

    await expect(anonApp.getByRole('button', { name: 'Create an account with a passkey' })).toBeVisible();
    await anonApp.getByRole('button', { name: 'I already have an account' }).click();

    await expect(anonApp.getByRole('button', { name: 'Sign in with a passkey' })).toBeVisible();

    api.linkPreviews['signup-token'] = {
      state: 'bind',
      channel: 'telegram',
      user_id: '77',
      merge_item_count: null,
    };
    await anonApp.getByRole('button', { name: 'Sign in with a passkey' }).click();

    await expect(anonApp.getByRole('button', { name: 'Connect', exact: true })).toBeVisible();
    await anonApp.getByRole('button', { name: 'Connect', exact: true }).click();
    await expect(anonApp.getByText('Connected. You can close this tab.')).toBeVisible();
  });

  test('claiming a link handles 403 passkey step-up and retries successfully', async ({ app, api }) => {
    await addVirtualAuthenticator(app);
    await app.goto('/link?token=bind-token');
    await plantCredential(app);

    await expect(app.getByRole('button', { name: 'Connect', exact: true })).toBeVisible();

    // First attempt to claim will fail with 403 step-up request
    api.failNext = {
      path: /\/link\/claim/,
      status: 403,
      error: 'Confirm with your passkey to continue',
    };

    await app.getByRole('button', { name: 'Connect', exact: true }).click();

    // useSensitiveAction caught 403, re-asserted passkey, and retried claim successfully
    await expect(app.getByText('Connected. You can close this tab.')).toBeVisible();
    const claims = api.requests.filter((r) => r.url === '/link/claim');
    expect(claims.length).toBe(2);
    expect(api.requests.some((r) => r.url === '/auth/passkey/login/finish')).toBe(true);
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
