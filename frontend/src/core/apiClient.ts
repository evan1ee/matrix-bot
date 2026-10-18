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

type ApiRequestOptions = RequestInit & {
  /**
   * Suppresses {@link AUTH_EXPIRED_EVENT} on a 401.
   */
  suppressAuthEvent?: boolean;
  /**
   * Optional client-side timeout in milliseconds (defaults to 90 seconds).
   */
  timeoutMs?: number;
};

/** Sends the HttpOnly session cookie with every request. */
const SESSION_CREDENTIALS: RequestCredentials = 'include';

/**
 * Headers every API call carries. A multipart body needs the browser to write
 * its own Content-Type, boundary included; forcing JSON here would make the
 * server reject the upload.
 */
function buildHeaders(body: BodyInit | null | undefined, extra?: HeadersInit): Record<string, string> {
  const defaults: Record<string, string> = body instanceof FormData ? {} : { 'Content-Type': 'application/json' };
  return { ...defaults, ...(extra as Record<string, string> | undefined) };
}

/**
 * Sends the request with the session cookie attached and returns the raw
 * response.
 *
 * There is no token to read or store any more. The session lives in an
 * HttpOnly cookie the browser sends automatically, which is why `credentials:
 * 'include'` is the whole of the auth logic here — and why an XSS on this page
 * cannot read the credential out and use it elsewhere.
 */
async function sendRequest(endpoint: string, options: ApiRequestOptions): Promise<Response> {
  const { timeoutMs = 90_000, suppressAuthEvent: _suppress, signal: callerSignal, ...fetchOptions } = options;

  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);

  if (callerSignal) {
    if (callerSignal.aborted) {
      controller.abort();
    } else {
      callerSignal.addEventListener('abort', () => controller.abort());
    }
  }

  const headers = buildHeaders(fetchOptions.body, fetchOptions.headers);

  try {
    return await fetch(`${API_BASE}${endpoint}`, {
      ...fetchOptions,
      headers,
      credentials: SESSION_CREDENTIALS,
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
}

/** The body of a response: JSON when the server sent JSON, otherwise `{ error }` holding a text excerpt. */
async function readBody(response: Response): Promise<any> {
  const contentType = response.headers.get('content-type');
  if (contentType && contentType.includes('application/json')) {
    try {
      return await response.json();
    } catch {
      return { error: `Malformed JSON response from server (Status: ${response.status})` };
    }
  }
  const rawText = await response.text();
  return { error: rawText.substring(0, 200) || `Server error (Status: ${response.status})` };
}

function rejectFailed(response: Response, data: any, options: ApiRequestOptions): never {
  if (response.status === 401 && !options.suppressAuthEvent) {
    // The session is gone — expired, revoked, or never established. Tell the
    // app so it can show the sign-in screen rather than looping on 401s.
    window.dispatchEvent(new Event(AUTH_EXPIRED_EVENT));
  }
  throw new ApiHttpError(data.error || `HTTP error! Status: ${response.status}`, response.status, data);
}

/**
 * Calls the API and returns the response headers alongside the body (a few
 * endpoints signal state in a header).
 */
export async function fetchApiResponse<T>(
  endpoint: string,
  options: ApiRequestOptions = {}
): Promise<{ data: T; headers: Headers }> {
  const response = await sendRequest(endpoint, options);

  if (response.status === 204) {
    return { data: {} as T, headers: response.headers };
  }

  const data = await readBody(response);
  if (!response.ok) rejectFailed(response, data, options);
  return { data: data as T, headers: response.headers };
}

/**
 * Downloads a binary response (a zip export). A failure carries the same
 * `{ error, code }` body as any other call.
 */
export async function fetchApiBlob(
  endpoint: string,
  options: ApiRequestOptions = {}
): Promise<{ blob: Blob; headers: Headers }> {
  const response = await sendRequest(endpoint, options);
  if (!response.ok) rejectFailed(response, await readBody(response), options);
  return { blob: await response.blob(), headers: response.headers };
}

/** [`fetchApiResponse`] for callers that only need the body. */
export async function fetchApi<T>(
  endpoint: string,
  options: Parameters<typeof fetchApiResponse>[1] = {}
): Promise<T> {
  return (await fetchApiResponse<T>(endpoint, options)).data;
}

/**
 * POSTs `body` and reads the `text/event-stream` answer, calling `onEvent`
 * with each `data:` line parsed as JSON. Resolves when the server ends the
 * stream, which can happen without a terminal event; the caller decides what
 * that means.
 *
 * There is no timeout: the server bounds a turn and keeps the connection alive
 * with comment lines. A caller's abort rejects with the browser's own
 * `AbortError`, unchanged, so Stop can be told apart from a failure.
 */
export async function streamApi<E>(
  endpoint: string,
  body: unknown,
  opts: { signal: AbortSignal; onEvent: (event: E) => void }
): Promise<void> {
  const payload = JSON.stringify(body);
  const response = await fetch(`${API_BASE}${endpoint}`, {
    method: 'POST',
    headers: buildHeaders(payload),
    body: payload,
    credentials: SESSION_CREDENTIALS,
    signal: opts.signal,
  });
  if (!response.ok) rejectFailed(response, await readBody(response), {});
  if (!response.body) throw new Error('This browser cannot read a streamed response');

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';

  const handleLine = (raw: string) => {
    const line = raw.endsWith('\r') ? raw.slice(0, -1) : raw;
    // Blank lines separate frames and `:` lines are keep-alive comments.
    if (!line || line.startsWith(':') || !line.startsWith('data:')) return;
    const value = line.slice(5).replace(/^ /, '');
    try {
      opts.onEvent(JSON.parse(value) as E);
    } catch (err) {
      if (err instanceof SyntaxError) throw new Error('Malformed event from the server');
      throw err;
    }
  };

  for (;;) {
    const { done, value } = await reader.read();
    // `stream: true` keeps a multibyte character split across chunks intact.
    buffer += decoder.decode(value, { stream: !done });
    let newline = buffer.indexOf('\n');
    while (newline >= 0) {
      handleLine(buffer.slice(0, newline));
      buffer = buffer.slice(newline + 1);
      newline = buffer.indexOf('\n');
    }
    if (done) break;
  }
  handleLine(buffer);
}
