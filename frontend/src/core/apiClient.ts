const API_BASE = '/api/v1';

export function getAuthToken(): string | null {
  return localStorage.getItem('matrix_bot_jwt');
}

export function setAuthToken(token: string | null) {
  if (token) {
    localStorage.setItem('matrix_bot_jwt', token);
  } else {
    localStorage.removeItem('matrix_bot_jwt');
  }
}

export function clearAuthToken() {
  localStorage.removeItem('matrix_bot_jwt');
}

export async function fetchApi<T>(
  endpoint: string,
  options: RequestInit = {}
): Promise<T> {
  const token = getAuthToken();
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
    ...(options.headers as Record<string, string>),
  };

  if (token) {
    headers['Authorization'] = `Bearer ${token}`;
  }

  const url = `${API_BASE}${endpoint}`;
  const response = await fetch(url, { ...options, headers });

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

  if (!response.ok) {
    throw new Error(data.error || `HTTP error! Status: ${response.status}`);
  }

  return data as T;
}
