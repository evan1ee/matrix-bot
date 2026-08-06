import { useEffect, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { registry } from './registry';
import { SystemEvent } from './types';
import { fetchApi, getAuthToken } from './apiClient';

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

    let eventSource: EventSource | null = null;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
    let attempt = 0;
    let closed = false;

    const scheduleReconnect = () => {
      const delay = Math.min(1000 * 2 ** attempt, 30000);
      attempt += 1;
      reconnectTimer = setTimeout(connect, delay);
    };

    // EventSource cannot send an Authorization header, so the stream URL has to
    // carry a credential. Exchange the long-lived API token for a 30-second
    // ticket that opens nothing else, rather than putting the real token into a
    // URL that proxies and browser history will record.
    const connect = async () => {
      if (closed) return;

      let ticket: string;
      try {
        const res = await fetchApi<{ ticket: string }>('/events/ticket', { method: 'POST' });
        ticket = res.ticket;
      } catch {
        setIsConnected(false);
        if (!closed) scheduleReconnect();
        return;
      }

      if (closed) return;

      eventSource = new EventSource(`/api/v1/events?ticket=${encodeURIComponent(ticket)}`);

      eventSource.onopen = () => {
        attempt = 0;
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

      // EventSource retries some failures itself, but not ones that close the
      // stream (an expired token, a server restart). Reconnect explicitly with
      // backoff so the UI recovers instead of silently going stale.
      eventSource.onerror = () => {
        setIsConnected(false);
        if (closed || eventSource?.readyState !== EventSource.CLOSED) return;

        eventSource.close();
        scheduleReconnect();
      };
    };

    void connect();

    return () => {
      closed = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      eventSource?.close();
      setIsConnected(false);
    };
  }, [queryClient, activeToken]);

  return { events, isConnected };
}
