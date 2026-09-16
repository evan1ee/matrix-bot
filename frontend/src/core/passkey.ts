import { fetchApi } from './apiClient';

/**
 * The browser side of the two WebAuthn ceremonies.
 *
 * The server speaks the WebAuthn wire format, which uses base64url wherever the
 * browser API uses `ArrayBuffer`. These converters are the whole of the
 * impedance mismatch — everything else is passed through untouched, so the
 * server's options reach `navigator.credentials` exactly as `webauthn-rs`
 * built them.
 */

export interface AccountSummary {
  account_id: string;
  display_name: string;
}

function base64UrlToBuffer(value: string): ArrayBuffer {
  const padded = value.replace(/-/g, '+').replace(/_/g, '/');
  const binary = atob(padded + '='.repeat((4 - (padded.length % 4)) % 4));
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes.buffer;
}

function bufferToBase64Url(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = '';
  for (let i = 0; i < bytes.length; i += 1) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

/** Whether this browser can do passkeys at all. */
export function isPasskeySupported(): boolean {
  return typeof window !== 'undefined' && !!window.PublicKeyCredential;
}

interface CeremonyStart {
  ceremony_id: string;
  options: any;
}

/** Turns the server's creation options into what the browser expects. */
function toCreationOptions(options: any): CredentialCreationOptions {
  const publicKey = options.publicKey ?? options;
  return {
    publicKey: {
      ...publicKey,
      challenge: base64UrlToBuffer(publicKey.challenge),
      user: {
        ...publicKey.user,
        id: base64UrlToBuffer(publicKey.user.id),
      },
      excludeCredentials: (publicKey.excludeCredentials ?? []).map((cred: any) => ({
        ...cred,
        id: base64UrlToBuffer(cred.id),
      })),
    },
  };
}

/** Turns the server's request options into what the browser expects. */
function toRequestOptions(options: any): CredentialRequestOptions {
  const publicKey = options.publicKey ?? options;
  return {
    publicKey: {
      ...publicKey,
      challenge: base64UrlToBuffer(publicKey.challenge),
      allowCredentials: (publicKey.allowCredentials ?? []).map((cred: any) => ({
        ...cred,
        id: base64UrlToBuffer(cred.id),
      })),
    },
  };
}

function serialiseRegistration(credential: PublicKeyCredential) {
  const response = credential.response as AuthenticatorAttestationResponse;
  return {
    id: credential.id,
    rawId: bufferToBase64Url(credential.rawId),
    type: credential.type,
    response: {
      attestationObject: bufferToBase64Url(response.attestationObject),
      clientDataJSON: bufferToBase64Url(response.clientDataJSON),
    },
  };
}

function serialiseAssertion(credential: PublicKeyCredential) {
  const response = credential.response as AuthenticatorAssertionResponse;
  return {
    id: credential.id,
    rawId: bufferToBase64Url(credential.rawId),
    type: credential.type,
    response: {
      authenticatorData: bufferToBase64Url(response.authenticatorData),
      clientDataJSON: bufferToBase64Url(response.clientDataJSON),
      signature: bufferToBase64Url(response.signature),
      userHandle: response.userHandle ? bufferToBase64Url(response.userHandle) : null,
    },
  };
}

/**
 * Creates an account and its first passkey.
 *
 * The account does not exist until this resolves — an abandoned ceremony leaves
 * nothing behind on the server.
 */
export async function registerPasskey(
  displayName: string,
  invite?: string
): Promise<AccountSummary> {
  const start = await fetchApi<CeremonyStart>('/auth/passkey/register/start', {
    method: 'POST',
    body: JSON.stringify({ display_name: displayName, invite: invite || null }),
  });

  const credential = (await navigator.credentials.create(
    toCreationOptions(start.options)
  )) as PublicKeyCredential | null;

  if (!credential) {
    throw new Error('No passkey was created');
  }

  return fetchApi<AccountSummary>('/auth/passkey/register/finish', {
    method: 'POST',
    body: JSON.stringify({
      ceremony_id: start.ceremony_id,
      credential: serialiseRegistration(credential),
      label: passkeyLabel(),
    }),
  });
}

/**
 * Signs in without a username.
 *
 * The credential itself carries the account handle, so the browser's own
 * picker is the account picker and nothing is typed.
 */
export async function signInWithPasskey(): Promise<AccountSummary> {
  const start = await fetchApi<CeremonyStart>('/auth/passkey/login/start', {
    method: 'POST',
  });

  const credential = (await navigator.credentials.get(
    toRequestOptions(start.options)
  )) as PublicKeyCredential | null;

  if (!credential) {
    throw new Error('No passkey was offered');
  }

  return fetchApi<AccountSummary>('/auth/passkey/login/finish', {
    method: 'POST',
    body: JSON.stringify({
      ceremony_id: start.ceremony_id,
      credential: serialiseAssertion(credential),
    }),
  });
}

export async function signOut(): Promise<void> {
  await fetchApi('/auth/logout', { method: 'POST' });
}

/**
 * Enrolls a passkey using a single-use token issued from Matrix chat (!passkey add).
 */
export async function enrollPasskey(token: string): Promise<AccountSummary> {
  const start = await fetchApi<CeremonyStart>('/auth/passkey/enroll/start', {
    method: 'POST',
    body: JSON.stringify({ token }),
  });

  const credential = (await navigator.credentials.create(
    toCreationOptions(start.options)
  )) as PublicKeyCredential | null;

  if (!credential) {
    throw new Error('No passkey was created');
  }

  return fetchApi<AccountSummary>('/auth/passkey/register/finish', {
    method: 'POST',
    body: JSON.stringify({
      ceremony_id: start.ceremony_id,
      credential: serialiseRegistration(credential),
      label: passkeyLabel(),
    }),
  });
}

/**
 * Adds another passkey to an already signed-in account.
 */
export async function addPasskey(): Promise<AccountSummary> {
  const start = await fetchApi<CeremonyStart>('/account/passkeys/add/start', {
    method: 'POST',
  });

  const credential = (await navigator.credentials.create(
    toCreationOptions(start.options)
  )) as PublicKeyCredential | null;

  if (!credential) {
    throw new Error('No passkey was created');
  }

  return fetchApi<AccountSummary>('/auth/passkey/register/finish', {
    method: 'POST',
    body: JSON.stringify({
      ceremony_id: start.ceremony_id,
      credential: serialiseRegistration(credential),
      label: passkeyLabel(),
    }),
  });
}

/**
 * A recognisable name for the device being registered.
 *
 * Best-effort and cosmetic: it only has to help someone spot which entry to
 * remove in their passkey list months later.
 */
function passkeyLabel(): string {
  const ua = navigator.userAgent;
  if (/iPhone|iPad/.test(ua)) return 'iOS device';
  if (/Android/.test(ua)) return 'Android device';
  if (/Mac OS X/.test(ua)) return 'Mac';
  if (/Windows/.test(ua)) return 'Windows PC';
  if (/Linux/.test(ua)) return 'Linux PC';
  return 'Passkey';
}

/**
 * Turns a WebAuthn failure into something worth showing a person.
 *
 * The browser deliberately gives the same `NotAllowedError` for "you cancelled"
 * and "that did not verify", so this cannot distinguish them — and says so
 * rather than guessing.
 */
export function describePasskeyError(error: unknown): string {
  if (error instanceof DOMException) {
    switch (error.name) {
      case 'NotAllowedError':
        return 'Passkey prompt was dismissed or timed out. Try again.';
      case 'InvalidStateError':
        return 'This device already has a passkey for this account. Sign in instead.';
      case 'NotSupportedError':
        return 'This device cannot create the kind of passkey this app needs.';
      case 'SecurityError':
        return 'Passkeys need a secure connection to this exact site.';
      default:
        return error.message || 'The passkey could not be used.';
    }
  }
  return error instanceof Error ? error.message : 'Something went wrong.';
}
