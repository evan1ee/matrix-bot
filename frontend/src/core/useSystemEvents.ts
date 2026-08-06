import { useEffect, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { registry } from './registry';
import { SystemEvent } from './types';
import { getAuthToken } from './apiClient';

export function useSystemEvents(token?: string | null) {
  const queryClient = useQueryClient();
  const [events, setEvents] = useState<SystemEvent[]>([]);
  const [isConnected, setIsConnected] = useState(false);

  const activeToken = token !== undefined ? token : getAuthToken();

  useEffect(() => {
    if (!activeToken) {
      setIsConnected(false);
      return;
    }

    const sseUrl = `/api/v1/events?token=${encodeURIComponent(activeToken)}`;
    const eventSource = new EventSource(sseUrl);

    eventSource.onopen = () => {
      setIsConnected(true);
    };

    eventSource.onmessage = (event) => {
      try {
        const rawData = JSON.parse(event.data);
        const sysEvent: SystemEvent = {
          type: rawData.type || 'Event',
          payload: rawData.payload !== undefined ? rawData.payload : rawData,
          timestamp: new Date().toLocaleTimeString(),
        };

        setEvents((prev) => [sysEvent, ...prev.slice(0, 49)]);

        // Notify all registered modules
        registry.getAll().forEach((mod) => {
          if (mod.handleSseEvent) {
            mod.handleSseEvent(sysEvent, queryClient);
          }
        });
      } catch (err) {
        console.error('Failed to parse SSE event:', err);
      }
    };

    eventSource.onerror = () => {
      setIsConnected(false);
    };

    return () => {
      eventSource.close();
    };
  }, [queryClient, activeToken]);

  return { events, isConnected };
}
