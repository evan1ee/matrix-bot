const API_BASE = '/api/v1';

/// Fired when the server rejects our session, so the app can drop its cached
/// identity and prompt for sign-in instead of retrying forever.
export const AUTH_EXPIRED_EVENT = 'matrixbot:auth-expired';

/**
 * Calls the API with the session cookie attached.
 *
 * There is no token to read or store any more. The session lives in an
 * HttpOnly cookie the browser sends automatically, which is why `credentials:
 * 'include'` is the whole of the auth logic here — and why an XSS on this page
 * cannot read the credential out and use it elsewhere.
 */
export async function fetchApi<T>(
  endpoint: string,
  options: RequestInit & {
    /**
     * Suppresses {@link AUTH_EXPIRED_EVENT} on a 401.
     *
     * Set by the session probe. Without it a signed-out page loops: the probe
     * 401s, the event invalidates the session query, the query refetches the
     * probe, and it 401s again — several requests a second, forever.
     */
    suppressAuthEvent?: boolean;
  } = {}
): Promise<T> {
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
    ...(options.headers as Record<string, string>),
  };

  const url = `${API_BASE}${endpoint}`;
  const response = await fetch(url, {
    ...options,
    headers,
    // Sends the session cookie. Also required for the browser to *store* the
    // cookie the sign-in endpoints set.
    credentials: 'include',
  });

  if (response.status === 204) {
    return {} as T;
  }

  let data: any;
  const contentType = response.headers.get('content-type');
  if (contentType && contentType.includes('application/json')) {
    try {
      data = await response.json();
    } catch {
      data = { error: `Malformed JSON response from server (Status: ${response.status})` };
    }
  } else {
    const rawText = await response.text();
    data = { error: rawText.substring(0, 200) || `Server error (Status: ${response.status})` };
  }

  if (response.status === 401 && !options.suppressAuthEvent) {
    // The session is gone — expired, revoked, or never established. Tell the
    // app so it can show the sign-in screen rather than looping on 401s.
    window.dispatchEvent(new Event(AUTH_EXPIRED_EVENT));
  }

  if (!response.ok) {
    throw new Error(data.error || `HTTP error! Status: ${response.status}`);
  }

  return data as T;
}
