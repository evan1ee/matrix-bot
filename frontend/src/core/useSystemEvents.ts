import { useEffect, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { registry } from './registry';
import { SystemEvent } from './types';

export function useSystemEvents() {
  const queryClient = useQueryClient();
  const [events, setEvents] = useState<SystemEvent[]>([]);
  const [isConnected, setIsConnected] = useState(false);

  useEffect(() => {
    const eventSource = new EventSource('/api/v1/events');

    eventSource.onopen = () => {
      setIsConnected(true);
    };

    eventSource.onmessage = (event) => {
      try {
        const rawData = JSON.parse(event.data);
        const sysEvent: SystemEvent = {
          type: rawData.type || 'Event',
          payload: rawData.data || rawData,
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
  }, [queryClient]);

  return { events, isConnected };
}
