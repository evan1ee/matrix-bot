import { useQuery } from '@tanstack/react-query';
import { ApiHttpError, fetchApi } from '../../core/apiClient';
import { useSettings } from '../../core/settings';

interface BriefingAudio {
  url: string;
  expires_in_seconds: number;
}

/** Today's date in the user's saved zone, as the server keys recordings. */
function todayInZone(timezone: string): string {
  try {
    return new Date().toLocaleDateString('en-CA', { timeZone: timezone });
  } catch {
    return new Date().toLocaleDateString('en-CA');
  }
}

/**
 * The link to the spoken morning briefing for `date` (default: today in the
 * user's zone). `data` is `null` when no recording exists, which is the normal
 * case until the first scheduled briefing has run with speech enabled.
 */
export function useBriefingAudio(date?: string) {
  const { timezone } = useSettings();
  const day = date ?? todayInZone(timezone);
  return useQuery<BriefingAudio | null>({
    queryKey: ['briefing-audio', day],
    queryFn: async () => {
      try {
        return await fetchApi<BriefingAudio>(`/briefing/audio?date=${day}`);
      } catch (err) {
        if (err instanceof ApiHttpError && err.status === 404) return null;
        throw err;
      }
    },
    // The link expires after 5 minutes; stay under that so a refetch on focus or
    // mount never hands out one about to lapse.
    staleTime: 3 * 60 * 1000,
    retry: false,
  });
}
