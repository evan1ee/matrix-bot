import { useQuery, useQueryClient } from '@tanstack/react-query';
import { ApiHttpError, fetchApi } from './apiClient';

export const SESSION_QUERY_KEY = ['session'];

export interface SessionUser {
  account_id: string;
  username: string;
  display_name: string | null;
  avatar_url: string | null;
  linked_mxids: string[];
}

/**
 * Who is signed in, or `null`.
 *
 * The session lives in an HttpOnly cookie the page cannot read, so "am I signed
 * in?" is a question only the server can answer — asking it is the sole source
 * of truth. There is deliberately no client-side token to inspect, and no way
 * for the page's idea of the session to drift from the server's.
 */
export function useSession() {
  const query = useQuery<SessionUser | null>({
    queryKey: SESSION_QUERY_KEY,
    queryFn: async () => {
      try {
        return await fetchApi<SessionUser>('/auth/me', { suppressAuthEvent: true });
      } catch (err) {
        // A 401 here is the ordinary signed-out state, not an error worth
        // retrying or surfacing.
        if (err instanceof ApiHttpError && err.status === 401) {
          return null;
        }
        // Temporary network drop or server restarting should not be treated
        // as signed-out so existing session state is not prematurely wiped out.
        throw err;
      }
    },
    staleTime: 60 * 1000,
    retry: (failureCount, error) => {
      // Never retry on explicit 401 Unauthorized.
      if (error instanceof ApiHttpError && error.status === 401) {
        return false;
      }
      return failureCount < 5;
    },
  });

  return {
    user: query.data ?? null,
    isSignedIn: !!query.data,
    isLoading: query.isLoading,
    isError: query.isError,
  };
}

/** Re-reads the session after signing in or out. */
export function useRefreshSession() {
  const queryClient = useQueryClient();
  return () => queryClient.invalidateQueries({ queryKey: SESSION_QUERY_KEY });
}
