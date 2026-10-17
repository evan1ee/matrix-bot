# WebAuthn Conditional UI (Passkey Autofill) Spec

**Date:** 2026-10-17  
**Status:** Draft  
**Scope:** Frontend (`frontend/src/core/passkey.ts`, `frontend/src/components/AuthModal.tsx`, `frontend/e2e/auth.spec.ts`)

---

## 1. Intent and Why

`matrix-bot` web client currently uses discoverable passkey authentication without passwords or usernames. However, sign-in currently requires an explicit button click ("Sign in") to trigger the modal system passkey prompt.

Modern browsers (Chromium, Safari on macOS/iOS, Firefox) support the **WebAuthn Conditional UI** standard (`mediation: 'conditional'`). This allows the browser's credential manager or OS password manager (1Password, iCloud Keychain, Windows Hello, Bitwarden) to display saved passkeys directly in an autofill dropdown when a user interacts with the sign-in form.

By implementing WebAuthn Conditional UI:
1. Returning users see their saved passkey immediately in the browser autofill dropdown upon opening the sign-in modal.
2. Selecting a passkey from the dropdown completes the WebAuthn assertion seamlessly in a single step without needing to click the manual "Sign in" button.
3. Users on browsers or platforms without Conditional UI support retain the existing explicit "Sign in" button without any disruption.

---

## 2. Goals, Non-Goals, and Anti-Goals

### Goals
- **Seamless Autofill:** Support WebAuthn Conditional Mediation (`mediation: 'conditional'`) in `frontend/src/core/passkey.ts` and `frontend/src/components/AuthModal.tsx`.
- **Autofill Input:** Present an accessible form input with `autoComplete="username webauthn"` when in `'signin'` mode, allowing browser credential managers to attach autofill suggestions.
- **Clean Cancellation:** Cancel any ongoing conditional mediation request cleanly via `AbortController` when:
  - The user clicks the explicit "Sign in" button.
  - The user closes or dismisses the modal (`Escape`, backdrop click, close `X`).
  - The user switches modes (`'register'`, `'recovery'`, `'enroll'`).
  - The component unmounts.
- **Fail-Safe Fallback:** If `PublicKeyCredential.isConditionalMediationAvailable()` returns `false`, or if conditional mediation is not supported, gracefully degrade to standard manual modal sign-in.
- **Design System Fidelity:** Maintain monochrome-first aesthetics, Geist typography, restrained form styling, and full keyboard accessibility in accordance with `Design.md`.

### Non-Goals
- Changing backend WebAuthn endpoints or ceremonies: `/api/v1/auth/passkey/login/start` already issues discoverable authentication options with empty `allowCredentials`, which is 100% compliant with Conditional UI.
- Introducing required usernames: The input remains an autofill anchor (`autoComplete="username webauthn"`). Users do not need to type a username manually to sign in.

### Anti-Goals
- Never let an aborted conditional request display an error toast or alert ("DOMException: The operation was aborted").
- Never allow a pending conditional request to block or conflict with an explicit manual "Sign in" button click.
- Never regress existing passkey registration, recovery code redemption, or Matrix token enrollment workflows.

---

## 3. Architecture & Mechanics

### 3.1 Conditional Mediation Availability Check
In `frontend/src/core/passkey.ts`:
```ts
export async function isConditionalMediationAvailable(): Promise<boolean> {
  if (typeof window === 'undefined' || !window.PublicKeyCredential) {
    return false;
  }
  if (typeof PublicKeyCredential.isConditionalMediationAvailable === 'function') {
    return PublicKeyCredential.isConditionalMediationAvailable();
  }
  return false;
}
```

### 3.2 Enhanced `signInWithPasskey`
Allow optional configuration `{ conditional?: boolean; signal?: AbortSignal }`:
```ts
export interface SignInOptions {
  conditional?: boolean;
  signal?: AbortSignal;
}

export async function signInWithPasskey(opts?: SignInOptions): Promise<AccountSummary> {
  const start = await fetchApi<CeremonyStart>('/auth/passkey/login/start', {
    method: 'POST',
    signal: opts?.signal,
  });

  const requestOptions = toRequestOptions(start.options);
  if (opts?.conditional) {
    (requestOptions as any).mediation = 'conditional';
  }
  if (opts?.signal) {
    (requestOptions as any).signal = opts.signal;
  }

  const credential = (await navigator.credentials.get(requestOptions)) as PublicKeyCredential | null;
  if (!credential) {
    throw new Error('No passkey was offered');
  }

  return fetchApi<AccountSummary>('/auth/passkey/login/finish', {
    method: 'POST',
    body: JSON.stringify({
      ceremony_id: start.ceremony_id,
      credential: serialiseAssertion(credential),
    }),
    signal: opts?.signal,
  });
}
```

### 3.3 Form Layout and Input in `AuthModal.tsx`
When `mode === 'signin'`, render a semantic form containing:
1. An autofill input with `autoComplete="username webauthn"`, `name="username"`, `type="text"`.
2. The explicit manual "Sign in" button.
3. Subtitle explaining: "Select your passkey from the autofill dropdown or click Sign in."

### 3.4 Lifecycle and Cancellation Flow
- **Mount / Switch to Sign-in:**
  1. Check `await isConditionalMediationAvailable()`.
  2. If available and modal is still open and in `'signin'` mode, create `const controller = new AbortController()`.
  3. Store controller in `conditionalAbortRef`.
  4. Call `signInWithPasskey({ conditional: true, signal: controller.signal })`.
  5. On success: `onSuccess(); onClose();`.
  6. On error: If error is an `AbortError` or `controller.signal.aborted`, ignore silently. Otherwise, ignore background autofill dismissals without popups.
- **Manual Click / Switch / Close:**
  1. `conditionalAbortRef.current?.abort()`.
  2. Clear ref.
  3. If manual "Sign in" clicked, execute modal `signInWithPasskey()` with standard loading spinner and error handling.

---

## 4. Testing & Verification

1. **Unit & Typecheck:** Run `npm --prefix frontend run build` (`tsc && vite build`).
2. **Playwright E2E Tests:**
   - In `frontend/e2e/auth.spec.ts`, verify that `autoComplete="username webauthn"` input renders properly on the sign-in dialog.
   - Verify that standard manual passkey sign-in continues to work cleanly with the new input present.
   - Verify that conditional autofill flow completes sign-in and closes the dialog.
3. **Regression Tests:** All existing e2e auth suites (`auth.spec.ts`) pass without failures.
