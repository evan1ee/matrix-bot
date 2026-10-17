# WebAuthn Conditional UI (Passkey Autofill) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement WebAuthn Conditional UI (`mediation: 'conditional'`) on the web client sign-in form for seamless passkey autofill.

**Architecture:** Extend `frontend/src/core/passkey.ts` to detect conditional mediation availability and pass `mediation: 'conditional'` alongside an `AbortSignal` to `navigator.credentials.get`. Update `frontend/src/components/AuthModal.tsx` to mount an accessible form with an `<input autoComplete="username webauthn">` field, start silent conditional mediation on mount, and cleanly abort on user mode switches, manual sign-in clicks, or modal dismissal.

**Tech Stack:** React 19, TypeScript, WebAuthn Level 3 (`PublicKeyCredential`), Playwright.

**Spec:** [`docs/superpowers/specs/2026-10-17-webauthn-conditional-ui.md`](file:///D:/01_Data/code/matrix-hello-bot/docs/superpowers/specs/2026-10-17-webauthn-conditional-ui.md)

## Global Constraints

- Monochrome-first styling, Geist Sans / Geist Mono typography, restrained UI matching `Design.md`.
- No new external npm dependencies; use standard WebAuthn API (`navigator.credentials.get`, `PublicKeyCredential.isConditionalMediationAvailable`) and DOM `AbortController`.
- Fail-closed security and graceful degradation: if conditional mediation is unavailable, manual modal sign-in operates without disruption.
- Clean cancellation: silent abort handling on modal close, mode switch, or manual sign-in; never surface `AbortError` to the user.
- Every task ends with a clean commit and passing verification.

## Review Focus

1. **Unsupported browsers / environments:** If `PublicKeyCredential.isConditionalMediationAvailable` is undefined or resolves to `false`, the sign-in modal must remain completely functional with manual "Sign in".
2. **Race between conditional mediation and manual sign-in button:** Clicking the explicit "Sign in" button must abort the pending conditional request prior to starting the modal request.
3. **Modal dismissal / backdrop click during active conditional request:** Closing the modal aborts the pending request cleanly with no error alerts.
4. **Mode switching:** Switching to `'register'` or `'recovery'` aborts the active conditional request without triggering error states.
5. **Abort error suppression:** `AbortError` or `signal.aborted` during conditional mediation must be silently ignored.

---

### Task 1: `core/passkey.ts` Conditional UI Support

**Files:**
- Modify: `frontend/src/core/passkey.ts`
- Test: `frontend/e2e/auth.spec.ts`

**Interfaces:**
- Consumes: `fetchApi`, `toRequestOptions`, `serialiseAssertion`
- Produces:
  - `export async function isConditionalMediationAvailable(): Promise<boolean>`
  - `export interface SignInOptions { conditional?: boolean; signal?: AbortSignal }`
  - `export async function signInWithPasskey(opts?: SignInOptions): Promise<AccountSummary>`

- [ ] **Step 1: Write the failing e2e test for conditional options support**

In `frontend/e2e/auth.spec.ts`, add a test verifying `signInWithPasskey` handles conditional mediation parameters without error.

```ts
test('passkey module exports conditional mediation detection and supports conditional sign-in options', async ({ anonApp }) => {
  await anonApp.goto('/');
  const isAvailableType = await anonApp.evaluate(async () => {
    const mod = await import('/src/core/passkey.ts');
    return typeof mod.isConditionalMediationAvailable;
  });
  expect(isAvailableType).toBe('function');
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm --prefix frontend run test:e2e -- auth.spec.ts`
Expected: FAIL (property `isConditionalMediationAvailable` does not exist on `core/passkey.ts`).

- [ ] **Step 3: Implement `isConditionalMediationAvailable` and `signInWithPasskey(opts)` in `frontend/src/core/passkey.ts`**

Implement:
1. `isConditionalMediationAvailable`:
```ts
export async function isConditionalMediationAvailable(): Promise<boolean> {
  if (typeof window === 'undefined' || !window.PublicKeyCredential) {
    return false;
  }
  if (typeof PublicKeyCredential.isConditionalMediationAvailable === 'function') {
    try {
      return await PublicKeyCredential.isConditionalMediationAvailable();
    } catch {
      return false;
    }
  }
  return false;
}
```
2. `SignInOptions` and update `signInWithPasskey(opts?: SignInOptions)`:
Pass `signal: opts?.signal` to `fetchApi` and add `mediation: 'conditional'` and `signal: opts?.signal` to `navigator.credentials.get` options when `opts?.conditional` is true.

- [ ] **Step 4: Run test and build to verify it passes**

Run:
```bash
npm --prefix frontend run build
npm --prefix frontend run test:e2e -- auth.spec.ts
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/core/passkey.ts frontend/e2e/auth.spec.ts
git commit -m "feat(passkey): add conditional mediation check and options to signInWithPasskey"
```

---

### Task 2: `AuthModal.tsx` Conditional Autofill & Lifecycle Cancellation

**Files:**
- Modify: `frontend/src/components/AuthModal.tsx`
- Test: `frontend/e2e/auth.spec.ts`

**Interfaces:**
- Consumes: `isConditionalMediationAvailable`, `signInWithPasskey` from `frontend/src/core/passkey.ts`
- Produces:
  - Autofill `<input name="username" autoComplete="username webauthn">` inside `AuthModal` in `'signin'` mode.
  - Background `AbortController` triggering conditional mediation on mount and aborting on unmount/mode change/manual sign in.

- [ ] **Step 1: Write the failing test for WebAuthn autofill input in `AuthModal`**

In `frontend/e2e/auth.spec.ts`, update `signs in with no username typed` or add a test that checks the autofill input presence:

```ts
test('signin modal renders passkey autofill input with username webauthn autocomplete', async ({ anonApp }) => {
  await addVirtualAuthenticator(anonApp);
  await anonApp.goto('/');
  const dialog = await openAuthModal(anonApp);
  const autofillInput = dialog.locator('input[autocomplete="username webauthn"]');
  await expect(autofillInput).toBeVisible();
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm --prefix frontend run test:e2e -- auth.spec.ts`
Expected: FAIL (`input[autocomplete="username webauthn"]` not found).

- [ ] **Step 3: Implement Conditional UI and input field in `frontend/src/components/AuthModal.tsx`**

1. Render `<input type="text" name="username" autoComplete="username webauthn" placeholder="Select a saved passkey or click Sign in" ... />` in `'signin'` mode.
2. In `useEffect` when `isOpen` and `mode === 'signin'`, if `isPasskeySupported()`, check `isConditionalMediationAvailable()`.
3. If true, instantiate `AbortController`, store in a `useRef`, and invoke `signInWithPasskey({ conditional: true, signal: controller.signal })`.
4. On success: invoke `onSuccess()` and `onClose()`.
5. On catch: if `controller.signal.aborted` or `err.name === 'AbortError'`, ignore silently.
6. In effect cleanup, call `controller.abort()`.
7. In `handleSignIn`, call `conditionalAbortRef.current?.abort()` before calling modal `signInWithPasskey()`.

- [ ] **Step 4: Run test and typecheck to verify it passes**

Run:
```bash
npm --prefix frontend run build
npm --prefix frontend run test:e2e -- auth.spec.ts
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/AuthModal.tsx frontend/e2e/auth.spec.ts
git commit -m "feat(auth): integrate WebAuthn conditional UI autofill in AuthModal"
```

---

### Task 3: Comprehensive E2E Tests for Conditional UI & Edge Cases

**Files:**
- Modify: `frontend/e2e/auth.spec.ts`

**Interfaces:**
- Consumes: Playwright CDP virtual authenticator (`WebAuthn.addVirtualAuthenticator`), `openAuthModal`, `expectSignedIn`

- [ ] **Step 1: Write test for seamless passkey sign-in via conditional mediation**

Add test in `frontend/e2e/auth.spec.ts`:
```ts
test('signs in seamlessly via WebAuthn conditional UI autofill', async ({ anonApp, api }) => {
  await addVirtualAuthenticator(anonApp);
  await anonApp.goto('/');

  // Register an account first so a passkey exists
  await registerPasskey(anonApp);

  const account = await openAuthModal(anonApp);
  await account.getByRole('button', { name: /sign out/i }).click();
  await expectSignedOut(anonApp);

  // Reopen modal — conditional UI attaches to the input
  const dialog = await openAuthModal(anonApp);
  const input = dialog.locator('input[autocomplete="username webauthn"]');
  await expect(input).toBeVisible();

  // Virtual authenticator automatically fulfills conditional mediation requests
  await expectSignedIn(anonApp);

  const finish = api.requests.find((r) => r.url === '/auth/passkey/login/finish')?.body as any;
  expect(finish.credential.response.signature).toBeTruthy();
});
```

- [ ] **Step 2: Run test to verify it passes or fails**

Run: `npm --prefix frontend run test:e2e -- auth.spec.ts`
Expected: Verify behavior under CDP virtual authenticator.

- [ ] **Step 3: Write test for clean cancellation on mode switch and modal close**

Verify that switching from `'signin'` to `'register'` or closing the dialog aborts the request cleanly without showing alert messages.

- [ ] **Step 4: Run all e2e auth specs and build verification**

Run:
```bash
npm --prefix frontend run build
npm --prefix frontend run test:e2e -- auth.spec.ts
```
Expected: All tests PASS.

- [ ] **Step 5: Commit**

```bash
git add frontend/e2e/auth.spec.ts
git commit -m "test(auth): add e2e tests for WebAuthn conditional UI autofill and cancellation"
```

---

### Task 4: Documentation & Hand-Off Update

**Files:**
- Modify: `docs/superpowers/HANDOFF.md`

- [ ] **Step 1: Update Track 2 item 4 in `HANDOFF.md`**

Update Track 2 item 4 in `docs/superpowers/HANDOFF.md` to record completion of WebAuthn Conditional UI (Passkey Autofill).

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/HANDOFF.md
git commit -m "docs(superpowers): resolve WebAuthn conditional UI in handoff"
```
