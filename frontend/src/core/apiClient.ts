const API_BASE = '/api/v1';

/// Fired when the server rejects our session, so the app can drop its cached
/// identity and prompt for sign-in instead of retrying forever.
export const AUTH_EXPIRED_EVENT = 'matrixbot:auth-expired';

export class ApiHttpError extends Error {
  status: number;
  data: any;

  constructor(message: string, status: number, data?: any) {
    super(message);
    this.name = 'ApiHttpError';
    this.status = status;
    this.data = data;
  }
}

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
     */
    suppressAuthEvent?: boolean;
    /**
     * Optional client-side timeout in milliseconds (defaults to 90 seconds).
     */
    timeoutMs?: number;
  } = {}
): Promise<T> {
  const { timeoutMs = 90_000, suppressAuthEvent, signal: callerSignal, ...fetchOptions } = options;

  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);

  if (callerSignal) {
    if (callerSignal.aborted) {
      controller.abort();
    } else {
      callerSignal.addEventListener('abort', () => controller.abort());
    }
  }

  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
    ...(fetchOptions.headers as Record<string, string>),
  };

  const url = `${API_BASE}${endpoint}`;
  let response: Response;
  try {
    response = await fetch(url, {
      ...fetchOptions,
      headers,
      credentials: 'include',
      signal: controller.signal,
    });
  } catch (err: any) {
    if (err?.name === 'AbortError') {
      throw new Error(`Request timed out after ${Math.round(timeoutMs / 1000)} seconds`);
    }
    throw err;
  } finally {
    clearTimeout(timer);
  }

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
    throw new ApiHttpError(data.error || `HTTP error! Status: ${response.status}`, response.status, data);
  }

  return data as T;
}
