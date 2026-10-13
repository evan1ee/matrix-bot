import { useEffect, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { registry } from './registry';
import { SystemEvent } from './types';
import { fetchApi } from './apiClient';
import {
  DEFAULT_SETTINGS,
  formatTimeInZone,
  SETTINGS_QUERY_KEY,
  UserSettings,
} from './settings';
import { showToast } from './toast';


function notifyBrowser(
  title: string,
  body: string,
  tag: string,
  moduleId?: string
) {
  if (typeof window === 'undefined' || !('Notification' in window)) return;
  if (Notification.permission === 'granted') {
    try {
      const n = new Notification(title, { body, tag });
      if (moduleId) {
        n.onclick = () => {
          window.focus();
          window.dispatchEvent(
            new CustomEvent('app:navigate', { detail: { moduleId } })
          );
          n.close();
        };
      }
    } catch {
      // Ignore environments where Notification constructor is restricted
    }
  }
}

export function useSystemEvents(signedIn: boolean) {
  const queryClient = useQueryClient();
  const [events, setEvents] = useState<SystemEvent[]>([]);
  const [isConnected, setIsConnected] = useState(false);

  useEffect(() => {
    if (!signedIn) {
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

    // `EventSource` cannot set headers, which is why this used to exchange the
    // API token for a short-lived ticket passed through the URL. It *does* send
    // cookies, so the session rides along on its own and the whole ticket
    // mechanism is gone — along with the credential-in-a-URL problem it existed
    // to contain.
    const connect = async () => {
      if (closed) return;

      eventSource = new EventSource('/api/v1/events', { withCredentials: true });

      eventSource.onopen = () => {
        attempt = 0;
        setIsConnected(true);
      };

      eventSource.onmessage = (event) => {
        try {
          const rawData = JSON.parse(event.data);
          // Read from the cache rather than through `useSettings`, so a settings
          // change does not tear down and reopen the stream.
          const timezone =
            queryClient.getQueryData<UserSettings>(SETTINGS_QUERY_KEY)?.timezone ??
            DEFAULT_SETTINGS.timezone;

          const sysEvent: SystemEvent = {
            type: rawData.type || 'Event',
            payload: rawData.payload !== undefined ? rawData.payload : rawData,
            timestamp: formatTimeInZone(new Date().toISOString(), timezone),
          };

          setEvents((prev) => [sysEvent, ...prev.slice(0, 49)]);

          // Settings are core, not a module: every module formats against them,
          // so a change made in another tab has to land in the shared cache
          // before the modules are notified.
          if (sysEvent.type === 'SettingsUpdated') {
            const { settings } = sysEvent.payload as { settings: UserSettings };
            queryClient.setQueryData(SETTINGS_QUERY_KEY, settings);
          }

          // Central React Query cache invalidation mapping
          if (sysEvent.type.startsWith('Agent')) {
            queryClient.invalidateQueries({ queryKey: ['agent-runs'] });
            queryClient.invalidateQueries({ queryKey: ['agent-approvals'] });
            queryClient.invalidateQueries({ queryKey: ['agent-tasks'] });
            const payload = sysEvent.payload as Record<string, unknown> | undefined;
            if (payload?.run_id) {
              queryClient.invalidateQueries({ queryKey: ['agent-run-detail', payload.run_id] });
            }

            if (sysEvent.type === 'AgentApprovalRequired') {
              const toolName = (payload?.tool_name as string) || 'sensitive action';
              const desc = `Agent requested permission to execute: ${toolName}`;
              showToast({
                title: 'Agent Approval Required',
                description: desc,
                type: 'warning',
                action: {
                  label: 'Review',
                  onClick: () => {
                    window.dispatchEvent(
                      new CustomEvent('app:navigate', { detail: { moduleId: 'agent' } })
                    );
                  },
                },
              });
              notifyBrowser(
                'Agent Approval Required',
                desc,
                `agent-approval-${String(payload?.run_id ?? toolName)}`,
                'agent'
              );
            } else if (sysEvent.type === 'AgentRunCompleted') {
              const status = payload?.status as string;
              showToast({
                title: status === 'completed' ? 'Agent Run Completed' : 'Agent Run Failed',
                description: (payload?.error as string) || `Run completed using ${payload?.total_tokens ?? 0} tokens`,
                type: status === 'completed' ? 'success' : 'error',
              });
            }
          } else if (sysEvent.type.startsWith('Todo')) {
            queryClient.invalidateQueries({ queryKey: ['todos'] });
          } else if (sysEvent.type.startsWith('Reminder')) {
            queryClient.invalidateQueries({ queryKey: ['reminders'] });
            if (sysEvent.type === 'ReminderTriggered') {
              const payload = sysEvent.payload as Record<string, unknown> | undefined;
              const desc =
                (payload?.title as string) ||
                (payload?.message as string) ||
                'A scheduled reminder was triggered.';
              showToast({
                title: 'Reminder Alert',
                description: desc,
                type: 'info',
                action: {
                  label: 'View',
                  onClick: () => {
                    window.dispatchEvent(
                      new CustomEvent('app:navigate', { detail: { moduleId: 'reminder' } })
                    );
                  },
                },
              });
              notifyBrowser(
                'Reminder Alert',
                desc,
                `reminder-${String(payload?.id ?? desc)}`,
                'reminder'
              );
            }
          } else if (sysEvent.type === 'MorningBriefingSent') {
            const payload = sysEvent.payload as Record<string, unknown> | undefined;
            const desc =
              (payload?.summary as string) || 'Your daily morning briefing is ready.';
            showToast({
              title: 'Morning Briefing',
              description: desc,
              type: 'info',
            });
            notifyBrowser('Morning Briefing', desc, `briefing-${desc}`);
          } else if (sysEvent.type.startsWith('Transaction') || sysEvent.type.startsWith('Category')) {
            queryClient.invalidateQueries({ queryKey: ['transactions'] });
            queryClient.invalidateQueries({ queryKey: ['categories'] });
            queryClient.invalidateQueries({ queryKey: ['finance-summary'] });
          } else if (sysEvent.type.startsWith('Portfolio')) {
            queryClient.invalidateQueries({ queryKey: ['portfolio-holdings'] });
            queryClient.invalidateQueries({ queryKey: ['portfolio-summary'] });
            queryClient.invalidateQueries({ queryKey: ['portfolio-cash'] });
            queryClient.invalidateQueries({ queryKey: ['portfolio-alerts'] });
            if (sysEvent.type === 'PortfolioAlertTriggered') {
              const payload = sysEvent.payload as Record<string, unknown> | undefined;
              const symbol = (payload?.symbol as string) || 'Holding';
              const desc = `Price alert triggered for ${symbol}.`;
              showToast({
                title: 'Price Alert Triggered',
                description: desc,
                type: 'info',
                action: {
                  label: 'View',
                  onClick: () => {
                    window.dispatchEvent(
                      new CustomEvent('app:navigate', { detail: { moduleId: 'portfolio' } })
                    );
                  },
                },
              });
              notifyBrowser(
                'Price Alert Triggered',
                desc,
                `portfolio-alert-${String(payload?.id ?? symbol)}`,
                'portfolio'
              );
            }
          } else if (sysEvent.type.startsWith('RecurringBill')) {
            queryClient.invalidateQueries({ queryKey: ['recurring-bills'] });
          } else if (sysEvent.type.startsWith('Webhook')) {
            queryClient.invalidateQueries({ queryKey: ['webhooks'] });
          } else if (sysEvent.type === 'IdentityLinked') {
            queryClient.invalidateQueries({ queryKey: ['identities'] });
          }

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
  }, [queryClient, signedIn]);

  return { events, isConnected };
}
