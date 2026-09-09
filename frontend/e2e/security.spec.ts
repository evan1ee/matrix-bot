import { test, expect, gotoApp } from './fixtures';
import type { Page } from '@playwright/test';

/**
 * Settings → Security: passkeys, linked Matrix accounts, sessions, recovery,
 * and the audit log.
 *
 * This is the surface with the most security weight in the app and, until this
 * file, none of it had ever been rendered in a browser during a test.
 *
 * The step-up specs are the important ones. A sensitive action answers 403 when
 * the passkey assertion has gone stale, and `useSensitiveAction` is supposed to
 * treat that not as an error to show but as a request to re-prompt and retry.
 * That behaviour is invisible to the Rust tests — they can only prove the
 * server sends the 403 — and it is the difference between "occasionally asks
 * for your passkey" and "randomly fails with a permissions error".
 */

/** Chrome's virtual authenticator, so a real re-prompt can succeed. */
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

/**
 * Puts a discoverable credential on the virtual authenticator.
 *
 * The `app` fixture is signed in by the fake API answering `/auth/me`, so no
 * ceremony has run and the authenticator is empty. A step-up re-prompt calls
 * `navigator.credentials.get()`, which needs something to offer — without this
 * the retry cannot succeed and the test would be measuring an empty keyring
 * rather than the retry logic.
 */
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

/** Opens Settings and switches to the Security tab. */
async function openSecurity(page: Page) {
  await gotoApp(page);
  await page.locator('button[title="Settings"]:visible').first().click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('button', { name: /^security$/i }).click();
  return dialog;
}

test.describe('passkeys', () => {
  test('lists the account’s passkeys', async ({ app, api }) => {
    api.passkeys = [
      { id: 'cred_1', label: 'MacBook Touch ID', created_at: '2026-01-01T10:00:00Z', last_used_at: '2026-08-01T09:00:00Z' },
      { id: 'cred_2', label: 'YubiKey', created_at: '2026-02-02T10:00:00Z', last_used_at: null },
    ];

    const dialog = await openSecurity(app);
    await expect(dialog.getByText('MacBook Touch ID')).toBeVisible();
    await expect(dialog.getByText('YubiKey')).toBeVisible();
  });

  test('refuses to remove the only passkey, and says why', async ({ app, api }) => {
    api.passkeys = [
      { id: 'cred_only', label: 'Only key', created_at: '2026-01-01T10:00:00Z', last_used_at: null },
    ];

    const dialog = await openSecurity(app);
    // The UI warns before you even try.
    await expect(dialog.getByText(/only passkey/i)).toBeVisible();

    await dialog.getByRole('button', { name: /remove only key/i }).click();

    // And the server's refusal is surfaced rather than swallowed.
    await expect(dialog.getByRole('alert')).toContainText(/only passkey/i);
    await expect(dialog.getByText('Only key')).toBeVisible();
  });

  test('removes a passkey once a second one exists', async ({ app, api }) => {
    api.passkeys = [
      { id: 'cred_1', label: 'MacBook Touch ID', created_at: '2026-01-01T10:00:00Z', last_used_at: null },
      { id: 'cred_2', label: 'YubiKey', created_at: '2026-02-02T10:00:00Z', last_used_at: null },
    ];

    const dialog = await openSecurity(app);
    await dialog.getByRole('button', { name: /remove yubikey/i }).click();

    await expect(dialog.getByText('YubiKey')).toHaveCount(0);
    await expect(dialog.getByText('MacBook Touch ID')).toBeVisible();
  });
});

test.describe('linking a Matrix account', () => {
  test('shows the exact line to send, and that it is single-use', async ({ app }) => {
    const dialog = await openSecurity(app);
    await dialog.getByRole('button', { name: /link a matrix account/i }).click();

    // The user is about to retype this into a different app, so the whole
    // command is shown rather than the bare code.
    await expect(dialog.getByText('!link A3F9-K2QX')).toBeVisible();
    await expect(dialog.getByText(/expires in 5 minutes.*works once/i)).toBeVisible();
    // The rule that will otherwise surprise them.
    await expect(dialog.getByText(/shared room cancels it/i)).toBeVisible();
  });

  test('lists linked accounts and unlinks one', async ({ app, api }) => {
    api.linkedMxids = ['@evan:mbm.one', '@evan:work.org'];

    const dialog = await openSecurity(app);
    await expect(dialog.getByText('@evan:mbm.one')).toBeVisible();
    await expect(dialog.getByText('@evan:work.org')).toBeVisible();

    await dialog.getByRole('button', { name: /unlink @evan:work\.org/i }).click();

    await expect(dialog.getByText('@evan:work.org')).toHaveCount(0);
    await expect(dialog.getByText('@evan:mbm.one')).toBeVisible();
  });

  test('says so when nothing is linked', async ({ app, api }) => {
    api.linkedMxids = [];
    const dialog = await openSecurity(app);
    await expect(dialog.getByText(/no matrix account linked/i)).toBeVisible();
  });
});

test.describe('merging an account that already has data', () => {
  test('shows what will move before anything moves', async ({ app, api }) => {
    api.pendingMerge = {
      from_account: 'acct_old',
      preview: { todos: 47, reminders: 8, transactions: 132, recurring_bills: 0,
                 portfolio_holdings: 3, portfolio_cash_balances: 0,
                 finance_categories: 0, user_settings: 1 },
    };

    const dialog = await openSecurity(app);

    // Counts, so the decision is made with the numbers in view.
    await expect(dialog.getByText(/47 todos/i)).toBeVisible();
    await expect(dialog.getByText(/132 transactions/i)).toBeVisible();
    await expect(dialog.getByText(/3 portfolio holdings/i)).toBeVisible();
    // Empty tables are not listed as "0 bills".
    await expect(dialog.getByText(/recurring bills/i)).toHaveCount(0);

    await expect(dialog.getByText(/nothing is deleted/i)).toBeVisible();
  });

  test('confirming moves the data and clears the prompt', async ({ app, api }) => {
    api.pendingMerge = {
      from_account: 'acct_old',
      preview: { todos: 2, reminders: 0, transactions: 0, recurring_bills: 0,
                 portfolio_holdings: 0, portfolio_cash_balances: 0,
                 finance_categories: 0, user_settings: 0 },
    };

    const dialog = await openSecurity(app);
    await dialog.getByRole('button', { name: /move it into my account/i }).click();

    await expect(dialog.getByText(/waiting to link/i)).toHaveCount(0);
    expect(api.requests.some((r) => r.url === '/account/matrix/confirm-merge')).toBe(true);
  });
});

test.describe('sessions', () => {
  test('marks which device is this one', async ({ app, api }) => {
    api.sessions = [
      { id: 'sess_current', created_at: '2026-08-01T10:00:00Z', last_seen_at: '2026-08-12T09:00:00Z', user_agent: 'Chrome on macOS', current: true },
      { id: 'sess_other', created_at: '2026-07-01T10:00:00Z', last_seen_at: '2026-08-10T09:00:00Z', user_agent: 'Firefox on Linux', current: false },
    ];

    const dialog = await openSecurity(app);
    await expect(dialog.getByText('Chrome on macOS')).toBeVisible();
    await expect(dialog.getByText('this device', { exact: true })).toBeVisible();
    await expect(dialog.getByText('Firefox on Linux')).toBeVisible();
  });

  test('signing out other devices leaves this one signed in', async ({ app, api }) => {
    api.sessions = [
      { id: 'sess_current', created_at: '2026-08-01T10:00:00Z', last_seen_at: '2026-08-12T09:00:00Z', user_agent: 'Chrome on macOS', current: true },
      { id: 'sess_other', created_at: '2026-07-01T10:00:00Z', last_seen_at: '2026-08-10T09:00:00Z', user_agent: 'Firefox on Linux', current: false },
    ];

    const dialog = await openSecurity(app);
    await dialog.getByRole('button', { name: /sign out other devices/i }).click();

    await expect(dialog.getByText('Firefox on Linux')).toHaveCount(0);
    await expect(dialog.getByText('Chrome on macOS')).toBeVisible();
    // Still signed in here — the whole point of "other".
    await expect(app.getByRole('dialog')).toBeVisible();
  });
});

test.describe('recovery', () => {
  test('shows codes once, with a warning that they will not be shown again', async ({ app }) => {
    const dialog = await openSecurity(app);
    await dialog.getByRole('button', { name: /generate new codes/i }).click();

    await expect(dialog.getByText(/not shown again/i)).toBeVisible();
    await expect(dialog.getByText('CODE0-ABCDE')).toBeVisible();
    await expect(dialog.getByText('CODE9-ABCDE')).toBeVisible();
  });

  test('a pending recovery is an alarm, and can be cancelled', async ({ app, api }) => {
    api.recoveryStatus = { codes_remaining: 10, recovery_valid_after: '2026-08-13T09:00:00Z' };

    const dialog = await openSecurity(app);

    // It has to say what it means and what to do, because this may be someone
    // taking the account over.
    await expect(dialog.getByText(/account recovery was requested/i)).toBeVisible();
    await expect(dialog.getByText(/if this was not you/i)).toBeVisible();

    await dialog.getByRole('button', { name: /cancel recovery/i }).click();
    await expect(dialog.getByText(/account recovery was requested/i)).toHaveCount(0);
  });
});

test.describe('the audit log', () => {
  test('renders events, and flags the alarming ones differently', async ({ app, api }) => {
    api.securityEvents = [
      { id: 3, kind: 'recovery_requested', detail: 'requested from @evan:mbm.one', created_at: '2026-08-12T09:00:00Z' },
      { id: 2, kind: 'matrix_linked', detail: '@evan:mbm.one', created_at: '2026-08-11T09:00:00Z' },
      { id: 1, kind: 'signed_in', detail: null, created_at: '2026-08-10T09:00:00Z' },
    ];

    const dialog = await openSecurity(app);

    // Raw `kind` values are translated for a person.
    await expect(dialog.getByText('Account recovery requested')).toBeVisible();
    await expect(dialog.getByText('Matrix account linked', { exact: true })).toBeVisible();
    await expect(dialog.getByText('Signed in')).toBeVisible();
    await expect(dialog.getByText('requested from @evan:mbm.one')).toBeVisible();
  });
});

test.describe('step-up', () => {
  test('a stale session re-prompts for the passkey and retries, rather than erroring', async ({ app, api }) => {
    await addVirtualAuthenticator(app);
    await gotoApp(app);
    await plantCredential(app);

    const dialog = await openSecurity(app);

    // The server says the passkey assertion has gone stale. This is not an
    // error to show — it is a request to prove possession again.
    api.failNext = {
      path: /account\/recovery-codes/,
      status: 403,
      error: 'Confirm with your passkey to continue',
    };

    await dialog.getByRole('button', { name: /generate new codes/i }).click();

    // The retry succeeded, so the codes appear and no error is shown.
    await expect(dialog.getByText('CODE0-ABCDE')).toBeVisible();
    await expect(dialog.getByRole('alert')).toHaveCount(0);

    // And it really did re-prove possession rather than just retrying blind.
    expect(api.requests.some((r) => r.url === '/auth/passkey/login/finish')).toBe(true);
  });

  test('a failure that is not a step-up request is reported', async ({ app, api }) => {
    const dialog = await openSecurity(app);

    api.failNext = {
      path: /account\/recovery-codes/,
      status: 500,
      error: 'Something went wrong on the server',
    };

    await dialog.getByRole('button', { name: /generate new codes/i }).click();

    await expect(dialog.getByRole('alert')).toContainText(/something went wrong/i);
  });
});
