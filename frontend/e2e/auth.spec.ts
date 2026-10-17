import { test, expect, DASHBOARD_READY } from './fixtures';
import type { Page } from '@playwright/test';

/**
 * Sign-in is passkeys only, so these drive a **real** WebAuthn ceremony through
 * Chrome's virtual authenticator rather than stubbing `navigator.credentials`.
 *
 * That distinction is the point of the file. The server half — verifying an
 * assertion, checking the origin, minting a session — is covered by the Rust
 * tests and cannot be exercised here anyway, because the fake API answers with
 * canned challenges. What is *only* testable here is the browser half:
 * `core/passkey.ts` converting base64url to `ArrayBuffer` and back, mapping the
 * server's options onto what `navigator.credentials` expects, and how the UI
 * reacts. Stubbing the browser API would test none of it.
 *
 * The authenticator sets `hasResidentKey` and `isUserVerified` because sign-in
 * is usernameless: without a discoverable credential the browser has nothing to
 * offer and `navigator.credentials.get` finds no match.
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

/**
 * Brings up the auth dialog.
 *
 * Signed out, the app opens it on its own — `AUTH_EXPIRED_EVENT` fires as soon
 * as `/auth/me` comes back 401 — and its overlay swallows clicks aimed at the
 * sidebar behind it. Clicking unconditionally therefore retries until the test
 * times out, which reads as a hung suite rather than a failing assertion.
 */
async function openAuthModal(page: Page) {
  const dialog = page.getByRole('dialog');

  // It opens on its own once `/auth/me` answers 401, which is a network round
  // trip after load — so this waits rather than sampling `isVisible()` once and
  // racing it.
  const openedItself = await dialog
    .waitFor({ state: 'visible', timeout: 3000 })
    .then(() => true)
    .catch(() => false);
  if (openedItself) return dialog;

  // Targeted by `title`, which the sidebar sets deliberately and which does not
  // change with collapsed/expanded state or display name. A name regex matched
  // whichever button happened to come first and opened the wrong view.
  // `:visible` because the shell renders a desktop and a mobile sidebar, only
  // one of which is on screen — `.first()` alone picks the hidden one.
  await page
    .locator('button[title="Sign In"]:visible, button[title^="Logged in as"]:visible')
    .first()
    .click();
  await expect(dialog).toBeVisible();
  return dialog;
}

async function registerPasskey(page: Page, name = 'tester') {
  const dialog = await openAuthModal(page);
  await dialog.getByRole('button', { name: /create an account instead/i }).click();
  await page.getByLabel('Display name').fill(name);
  await dialog.getByRole('button', { name: /create passkey/i }).click();
  await expectSignedIn(page);
}

/**
 * Waits for the session to actually exist.
 *
 * Deliberately *not* the dashboard: the shell and its stat tiles render while
 * signed out too, with the auth dialog on top. Asserting on them passes
 * instantly and lets the rest of a test run before the ceremony has finished —
 * which is exactly the race that made this file look like it was hanging.
 * The dialog closing is the first thing that only happens once signed in.
 */
async function expectSignedIn(page: Page) {
  await expect(page.getByRole('dialog')).toBeHidden();
}

/** Waits until the app has stopped believing there is a session. */
async function expectSignedOut(page: Page) {
  await expect(page.locator('button[title="Sign In"]:visible').first()).toBeVisible();
}

test('registers an account with a passkey and lands signed in', async ({ anonApp, api }) => {
  await addVirtualAuthenticator(anonApp);
  await anonApp.goto('/');

  await registerPasskey(anonApp);
  await expect(anonApp.getByText(DASHBOARD_READY)).toBeVisible();

  // Both halves of the ceremony ran, in order.
  const ceremony = api.requests.filter((r) => r.url.startsWith('/auth/passkey/register/'));
  expect(ceremony.map((r) => r.url)).toEqual([
    '/auth/passkey/register/start',
    '/auth/passkey/register/finish',
  ]);

  // The credential the browser produced was forwarded in the shape the server
  // parses. A conversion bug in `core/passkey.ts` surfaces right here.
  const finish = ceremony[1].body as any;
  expect(finish.ceremony_id).toBe('ceremony_register');
  expect(finish.credential.id).toBeTruthy();
  expect(finish.credential.type).toBe('public-key');
  expect(finish.credential.response.attestationObject).toBeTruthy();
  expect(finish.credential.response.clientDataJSON).toBeTruthy();
});

test('signs in with no username typed', async ({ anonApp, api }) => {
  await addVirtualAuthenticator(anonApp);
  await anonApp.goto('/');

  // Register first, so the authenticator holds a discoverable credential.
  await registerPasskey(anonApp);

  // Intercept credentials.get so conditional mediation waits for autofill interaction
  await anonApp.evaluate(() => {
    const orig = navigator.credentials.get.bind(navigator.credentials);
    navigator.credentials.get = (options) => {
      if (options?.mediation === 'conditional') {
        return new Promise((_, reject) => {
          options.signal?.addEventListener('abort', () => {
            reject(new DOMException('The operation was aborted.', 'AbortError'));
          });
        });
      }
      return orig(options);
    };
  });

  const account = await openAuthModal(anonApp);
  await account.getByRole('button', { name: /sign out/i }).click();

  // Wait for the session to actually be gone before reopening. Signing out
  // invalidates a cached query, so for a moment the app still believes it is
  // signed in and the dialog would come back showing the account view.
  await expectSignedOut(anonApp);

  const dialog = await openAuthModal(anonApp);
  await expect(dialog.getByRole('textbox')).toHaveAttribute('autocomplete', 'username webauthn');
  await dialog.getByRole('button', { name: /^sign in$/i }).click();
  await expectSignedIn(anonApp);

  const finish = api.requests.find((r) => r.url === '/auth/passkey/login/finish')?.body as any;
  expect(finish.credential.response.signature).toBeTruthy();
  expect(finish.credential.response.authenticatorData).toBeTruthy();
});

test('signing out ends the session server-side, not just in the page', async ({ app, api }) => {
  await app.goto('/');
  await expect(app.getByText(DASHBOARD_READY)).toBeVisible();

  const dialog = await openAuthModal(app);
  await dialog.getByRole('button', { name: /sign out/i }).click();

  // The app must ask the server to revoke: the cookie it cannot read *is* the
  // credential, so clearing something client-side would leave a live session.
  expect(api.requests.some((r) => r.url === '/auth/logout' && r.method === 'POST')).toBe(true);

  await app.reload();
  await expect(app.getByRole('dialog')).toBeVisible();
});

test('a rejected session reopens the prompt instead of retrying forever', async ({ app, api }) => {
  await app.goto('/');
  await expect(app.getByText(DASHBOARD_READY)).toBeVisible();

  // The session stops being accepted mid-visit — expired, or revoked from
  // another device.
  api.signedIn = false;
  api.failNext = { path: /^\/todos$/, status: 401, error: 'That session has expired' };
  await app.reload();

  await expect(app.getByRole('dialog')).toBeVisible();
});

test('a rejected sign-in is explained rather than swallowed', async ({ anonApp, api }) => {
  await addVirtualAuthenticator(anonApp);
  await anonApp.goto('/');

  // The server refuses the assertion — an unknown credential, or one whose
  // signature did not verify. Deliberately driven from the server rather than
  // by dismissing the real prompt: Chrome reports a dismissal only after the
  // ceremony times out, which would hang the suite rather than test anything.
  const dialog = await openAuthModal(anonApp);
  await expect(dialog.getByRole('textbox')).toHaveAttribute('autocomplete', 'username webauthn');
  api.failNext = { path: /passkey\/login\/start/, status: 401, error: 'Sign-in failed' };
  await dialog.getByRole('button', { name: /^sign in$/i }).click();

  await expect(anonApp.getByRole('alert')).toBeVisible();
  await expect(anonApp.getByRole('alert')).toContainText(/sign-in failed/i);

  // And it stays on the prompt rather than pretending to have worked.
  await expect(anonApp.getByRole('dialog')).toBeVisible();
});

test('the page never holds the session where a script could read it', async ({ app }) => {
  await app.goto('/');
  await expect(app.getByText(DASHBOARD_READY)).toBeVisible();

  // The whole reason the session moved to an HttpOnly cookie: an XSS on this
  // page must find nothing worth stealing. `document.cookie` is checked too —
  // a cookie missing HttpOnly would show up there.
  const exposed = await app.evaluate(() => ({
    local: Object.entries(window.localStorage).filter(([k, v]) =>
      /token|jwt|session|bearer/i.test(String(k) + String(v))
    ),
    session: Object.entries(window.sessionStorage).filter(([k, v]) =>
      /token|jwt|session|bearer/i.test(String(k) + String(v))
    ),
    cookie: document.cookie,
  }));

  expect(exposed.local).toEqual([]);
  expect(exposed.session).toEqual([]);
  expect(exposed.cookie).not.toContain('mb_session');
});

test('registers a passkey using an enrollment token (?enroll=...) from Matrix chat', async ({
  anonApp,
  api,
}) => {
  await addVirtualAuthenticator(anonApp);
  await anonApp.goto('/?enroll=valid_token_123');

  // Modal automatically opens with enrollment prompt
  const dialog = anonApp.getByRole('dialog');
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText(/Link Passkey/i)).toBeVisible();

  await dialog.getByRole('button', { name: /Activate passkey for this device/i }).click();

  // The dashboard renders behind the modal while signed out, so its text is no
  // proof that enrollment finished; the modal closing is.
  await expect(dialog).toBeHidden();
  await expect(anonApp.getByText(DASHBOARD_READY)).toBeVisible();

  // Both halves of enrollment ran
  const enrollStart = api.requests.find((r) => r.url === '/auth/passkey/enroll/start');
  expect(enrollStart).toBeTruthy();
  expect(enrollStart?.body).toEqual({ token: 'valid_token_123' });

  const enrollFinish = api.requests.find((r) => r.url === '/auth/passkey/register/finish');
  expect(enrollFinish).toBeTruthy();
});

test('passkey module exports conditional mediation detection and supports conditional sign-in options', async ({ anonApp }) => {
  await anonApp.goto('/');
  const isAvailableType = await anonApp.evaluate(async () => {
    const mod = await import('/src/core/passkey.ts');
    return typeof mod.isConditionalMediationAvailable;
  });
  expect(isAvailableType).toBe('function');
});

test('signin modal renders passkey autofill input with username webauthn autocomplete', async ({ anonApp }) => {
  await addVirtualAuthenticator(anonApp);
  await anonApp.goto('/');
  const dialog = await openAuthModal(anonApp);
  const autofillInput = dialog.locator('input[autocomplete="username webauthn"]');
  await expect(autofillInput).toBeVisible();
});

test('signs in seamlessly via WebAuthn conditional UI autofill', async ({ anonApp, api }) => {
  await addVirtualAuthenticator(anonApp);
  await anonApp.goto('/');

  // Register an account first so a passkey exists
  await registerPasskey(anonApp);

  // Delay conditional mediation slightly so the autofill input's presence is observable
  // before the virtual authenticator fulfills the request and closes the modal.
  await anonApp.evaluate(() => {
    const orig = navigator.credentials.get.bind(navigator.credentials);
    navigator.credentials.get = async (options) => {
      if (options?.mediation === 'conditional') {
        await new Promise((r) => setTimeout(r, 150));
      }
      return orig(options);
    };
  });

  const account = await openAuthModal(anonApp);
  await account.getByRole('button', { name: /sign out/i }).click();

  // Signing out causes the sign-in modal to open automatically with conditional UI attached
  const dialog = anonApp.getByRole('dialog');
  await expect(dialog).toBeVisible();
  const input = dialog.locator('input[autocomplete="username webauthn"]');
  await expect(input).toBeVisible();

  // Virtual authenticator automatically fulfills conditional mediation requests
  await expectSignedIn(anonApp);

  const finish = api.requests.find((r) => r.url === '/auth/passkey/login/finish')?.body as any;
  expect(finish.credential.response.signature).toBeTruthy();
});

test('cancels conditional UI cleanly when switching modes or closing the dialog', async ({ anonApp }) => {
  await addVirtualAuthenticator(anonApp);
  await anonApp.goto('/');

  const dialog = await openAuthModal(anonApp);
  const input = dialog.locator('input[autocomplete="username webauthn"]');
  await expect(input).toBeVisible();

  // Switching modes aborts active conditional mediation; abort errors must never produce alert popups
  await dialog.getByRole('button', { name: /create an account instead/i }).click();
  await expect(dialog.getByLabel(/display name/i)).toBeVisible();
  await expect(anonApp.getByRole('alert')).toHaveCount(0);

  // Switch back to sign-in
  await dialog.getByRole('button', { name: /already have an account/i }).click();
  await expect(input).toBeVisible();
  await expect(anonApp.getByRole('alert')).toHaveCount(0);

  // Switch to recovery
  await dialog.getByRole('button', { name: /recovery code/i }).click();
  await expect(dialog.getByLabel(/recovery code/i)).toBeVisible();
  await expect(anonApp.getByRole('alert')).toHaveCount(0);

  // Close dialog via Escape
  await anonApp.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  await expect(anonApp.getByRole('alert')).toHaveCount(0);
});

test('manual button sign-in while conditional mediation was active succeeds without conflict', async ({
  anonApp,
  api,
}) => {
  await addVirtualAuthenticator(anonApp);
  await anonApp.goto('/');

  await registerPasskey(anonApp);

  // Intercept credentials.get to keep conditional mediation pending until aborted
  await anonApp.evaluate(() => {
    const orig = navigator.credentials.get.bind(navigator.credentials);
    navigator.credentials.get = (options) => {
      if (options?.mediation === 'conditional') {
        return new Promise((_, reject) => {
          options.signal?.addEventListener('abort', () => {
            reject(new DOMException('The operation was aborted.', 'AbortError'));
          });
        });
      }
      return orig(options);
    };
  });

  const account = await openAuthModal(anonApp);
  await account.getByRole('button', { name: /sign out/i }).click();

  const dialog = anonApp.getByRole('dialog');
  await expect(dialog).toBeVisible();
  await expect(dialog.locator('input[autocomplete="username webauthn"]')).toBeVisible();

  // Click manual sign-in button while conditional mediation is pending
  await dialog.getByRole('button', { name: /^sign in$/i }).click();
  await expectSignedIn(anonApp);
  await expect(anonApp.getByRole('alert')).toHaveCount(0);

  const finish = api.requests.find((r) => r.url === '/auth/passkey/login/finish')?.body as any;
  expect(finish.credential.response.signature).toBeTruthy();
});


