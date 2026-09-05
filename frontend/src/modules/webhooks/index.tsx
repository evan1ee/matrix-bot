import React, { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import { FrontendModule, SystemEvent } from '../../core/types';
import {
  Webhook as WebhookIcon, Plus, Trash2, RefreshCw, Send,
  Radio, CheckCircle2, Globe, History
} from 'lucide-react';
import {
  WebhookItem,
  WebhookLogsResponse,
  CreatedWebhookResponse,
  JoinedRoomInfo,
} from './types';
import { SecretModal } from './SecretModal';
import { WebhookLogsModal } from './WebhookLogsModal';
import { CreateWebhookModal } from './CreateWebhookModal';

export * from './types';

const WebhooksView: React.FC<{
  events?: SystemEvent[];
  isConnected?: boolean;
}> = () => {
  const queryClient = useQueryClient();
  const [isCreateOpen, setIsCreateOpen] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);

  // Modal shown once when a token is generated/rotated
  const [secretModalData, setSecretModalData] = useState<CreatedWebhookResponse | null>(null);
  const [testStatus, setTestStatus] = useState<{ id: string; message: string; ok: boolean } | null>(null);

  // Logs viewer state
  const [activeLogsWebhook, setActiveLogsWebhook] = useState<WebhookItem | null>(null);

  // Queries
  const { data: webhooks = [], isLoading: isLoadingWebhooks, error: webhooksError } = useQuery({
    queryKey: ['webhooks'],
    queryFn: () => fetchApi<WebhookItem[]>('/account/webhooks'),
  });

  const { data: rooms = [] } = useQuery({
    queryKey: ['rooms'],
    queryFn: () => fetchApi<JoinedRoomInfo[]>('/rooms'),
  });

  const {
    data: logsData,
    isLoading: isLoadingLogs,
    refetch: refetchLogs,
  } = useQuery({
    queryKey: ['webhook-logs', activeLogsWebhook?.id],
    queryFn: () =>
      activeLogsWebhook
        ? fetchApi<WebhookLogsResponse>(`/account/webhooks/${activeLogsWebhook.id}/logs?limit=50`)
        : Promise.resolve({ items: [], total: 0, limit: 50, offset: 0 }),
    enabled: !!activeLogsWebhook,
  });

  const getRoomName = (roomId: string) => {
    const found = rooms.find((r) => r.room_id === roomId);
    return found?.name || roomId;
  };

  // Mutations
  const createMutation = useMutation({
    mutationFn: (body: { name: string; default_room_id: string }) =>
      fetchApi<CreatedWebhookResponse>('/account/webhooks', {
        method: 'POST',
        body: JSON.stringify(body),
      }),
    onSuccess: (data) => {
      queryClient.invalidateQueries({ queryKey: ['webhooks'] });
      setIsCreateOpen(false);
      setFormError(null);
      setSecretModalData(data);
    },
    onError: (err: any) => {
      setFormError(err.message || 'Failed to create webhook');
    },
  });

  const rotateMutation = useMutation({
    mutationFn: (id: string) =>
      fetchApi<CreatedWebhookResponse>(`/account/webhooks/${id}/rotate`, {
        method: 'POST',
      }),
    onSuccess: (data) => {
      queryClient.invalidateQueries({ queryKey: ['webhooks'] });
      setSecretModalData(data);
    },
  });

  const testMutation = useMutation({
    mutationFn: (id: string) =>
      fetchApi<{ status: string; outbox_id: string }>(`/account/webhooks/${id}/test`, {
        method: 'POST',
      }),
    onSuccess: (_, id) => {
      setTestStatus({ id, message: 'Test alert queued successfully!', ok: true });
      setTimeout(() => setTestStatus(null), 4000);
      queryClient.invalidateQueries({ queryKey: ['webhooks'] });
      if (activeLogsWebhook?.id === id) {
        refetchLogs();
      }
    },
    onError: (err: any, id) => {
      setTestStatus({ id, message: err.message || 'Failed to trigger test alert', ok: false });
      setTimeout(() => setTestStatus(null), 5000);
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) =>
      fetchApi<void>(`/account/webhooks/${id}`, {
        method: 'DELETE',
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['webhooks'] });
      if (activeLogsWebhook) {
        setActiveLogsWebhook(null);
      }
    },
  });

  const toggleActiveMutation = useMutation({
    mutationFn: ({ id, is_active }: { id: string; is_active: boolean }) =>
      fetchApi<WebhookItem>(`/account/webhooks/${id}`, {
        method: 'PATCH',
        body: JSON.stringify({ is_active }),
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['webhooks'] });
    },
  });

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4 border-b border-neutral-200 dark:border-neutral-800 pb-5">
        <div>
          <h1 className="text-2xl font-bold tracking-tight text-neutral-900 dark:text-neutral-100 flex items-center gap-2">
            <WebhookIcon className="w-6 h-6 text-neutral-700 dark:text-neutral-300" />
            Inbound Webhooks
          </h1>
          <p className="text-sm text-neutral-500 dark:text-neutral-400 mt-1">
            Trigger Matrix notifications from external servers, monitoring daemons, cron jobs, or Telegram bots.
          </p>
        </div>
        <button
          onClick={() => {
            setIsCreateOpen(true);
            setFormError(null);
          }}
          className="h-9 inline-flex items-center gap-2 px-3.5 bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 font-medium text-xs rounded-md hover:bg-neutral-800 dark:hover:bg-neutral-200 transition-colors shadow-xs cursor-pointer"
        >
          <Plus className="w-4 h-4" />
          Create Webhook
        </button>
      </div>

      {/* Main List Content */}
      {isLoadingWebhooks ? (
        <div className="py-12 text-center text-neutral-500 dark:text-neutral-400 text-sm">Loading webhooks...</div>
      ) : webhooksError ? (
        <div className="p-4 bg-red-50 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 rounded-lg text-red-800 dark:text-red-300 text-sm">
          Failed to load webhooks: {(webhooksError as Error).message}
        </div>
      ) : webhooks.length === 0 ? (
        <div className="text-center py-16 px-4 border border-dashed border-neutral-300 dark:border-neutral-800 rounded-xl bg-neutral-50/50 dark:bg-neutral-900/40">
          <div className="w-10 h-10 rounded-full bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400 flex items-center justify-center mx-auto mb-4">
            <Globe className="w-5 h-5" />
          </div>
          <h3 className="text-base font-semibold text-neutral-900 dark:text-neutral-100">No inbound webhooks configured</h3>
          <p className="text-sm text-neutral-500 dark:text-neutral-400 max-w-md mx-auto mt-1 mb-6">
            Create an inbound webhook endpoint to send server alerts, CI/CD statuses, and monitoring events directly into your Matrix rooms.
          </p>
          <button
            onClick={() => {
              setIsCreateOpen(true);
              setFormError(null);
            }}
            className="inline-flex items-center gap-2 px-4 py-2 bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 font-medium text-sm rounded-lg hover:bg-neutral-800 dark:hover:bg-neutral-200 transition-colors cursor-pointer"
          >
            <Plus className="w-4 h-4" />
            Create Your First Webhook
          </button>
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-4">
          {webhooks.map((hook) => (
            <div
              key={hook.id}
              className={`p-5 rounded-xl border transition-all ${
                hook.is_active
                  ? 'bg-white dark:bg-neutral-900 border-neutral-200 dark:border-neutral-800 shadow-xs'
                  : 'bg-neutral-50 dark:bg-neutral-900/40 border-neutral-200/60 dark:border-neutral-800/60 opacity-75'
              }`}
            >
              <div className="flex flex-col md:flex-row md:items-center md:justify-between gap-4">
                <div className="space-y-1.5 min-w-0">
                  <div className="flex items-center gap-2.5 flex-wrap">
                    <h3 className="font-semibold text-base text-neutral-900 dark:text-neutral-100 truncate">{hook.name}</h3>
                    <span
                      className={`inline-flex items-center gap-1.5 px-2 py-0.5 text-xs font-medium rounded ${
                        hook.is_active
                          ? 'bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border border-emerald-500/20'
                          : 'bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400 border border-neutral-200 dark:border-neutral-700'
                      }`}
                    >
                      <Radio className={`w-3 h-3 ${hook.is_active ? 'text-emerald-500' : ''}`} />
                      {hook.is_active ? 'Active' : 'Disabled'}
                    </span>
                    <span className="font-mono text-xs text-neutral-600 dark:text-neutral-400 bg-neutral-100 dark:bg-neutral-800 px-2 py-0.5 rounded-md border border-neutral-200 dark:border-neutral-700">
                      {hook.token_prefix}...
                    </span>
                  </div>

                  <div className="flex flex-wrap items-center gap-y-1 gap-x-4 text-xs text-neutral-500 dark:text-neutral-400">
                    <div>
                      Target Room:{' '}
                      <span className="font-medium text-neutral-800 dark:text-neutral-200">{getRoomName(hook.default_room_id)}</span>
                    </div>
                    <div>&bull;</div>
                    <div>
                      Delivered: <span className="font-semibold text-neutral-800 dark:text-neutral-200">{hook.total_delivered_count}</span>
                    </div>
                    <div>&bull;</div>
                    <div>
                      Failures: <span className="font-semibold text-neutral-800 dark:text-neutral-200">{hook.failure_count}</span>
                    </div>
                    {hook.last_triggered_at && (
                      <>
                        <div>&bull;</div>
                        <div>
                          Last Triggered:{' '}
                          <span className="text-neutral-800 dark:text-neutral-200">
                            {new Date(hook.last_triggered_at).toLocaleString()}
                          </span>
                        </div>
                      </>
                    )}
                  </div>
                </div>

                {/* Actions */}
                <div className="flex items-center gap-2 flex-wrap sm:flex-nowrap">
                  <button
                    onClick={() => setActiveLogsWebhook(hook)}
                    title="View message history and delivery logs"
                    className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium bg-neutral-100 hover:bg-neutral-200 dark:bg-neutral-800 dark:hover:bg-neutral-700 text-neutral-700 dark:text-neutral-200 rounded-lg transition-colors cursor-pointer"
                  >
                    <History className="w-3.5 h-3.5 text-neutral-500" />
                    History ({hook.total_delivered_count + hook.failure_count})
                  </button>

                  <button
                    onClick={() => testMutation.mutate(hook.id)}
                    disabled={testMutation.isPending || !hook.is_active}
                    title="Send synthetic test alert"
                    className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium bg-neutral-100 hover:bg-neutral-200 dark:bg-neutral-800 dark:hover:bg-neutral-700 text-neutral-700 dark:text-neutral-200 rounded-lg transition-colors disabled:opacity-50 cursor-pointer"
                  >
                    <Send className="w-3.5 h-3.5" />
                    Test Ping
                  </button>

                  <button
                    onClick={() => {
                      if (confirm(`Rotate secret token for "${hook.name}"? The previous token will immediately stop working.`)) {
                        rotateMutation.mutate(hook.id);
                      }
                    }}
                    disabled={rotateMutation.isPending}
                    title="Regenerate secret token"
                    className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium bg-neutral-100 hover:bg-neutral-200 dark:bg-neutral-800 dark:hover:bg-neutral-700 text-neutral-700 dark:text-neutral-200 rounded-lg transition-colors cursor-pointer"
                  >
                    <RefreshCw className={`w-3.5 h-3.5 ${rotateMutation.isPending ? 'animate-spin' : ''}`} />
                    Rotate Secret
                  </button>

                  <button
                    onClick={() => toggleActiveMutation.mutate({ id: hook.id, is_active: !hook.is_active })}
                    title={hook.is_active ? 'Disable webhook' : 'Enable webhook'}
                    className="px-3 py-1.5 text-xs font-medium text-neutral-600 hover:text-neutral-900 dark:text-neutral-400 dark:hover:text-neutral-100 bg-neutral-100/60 hover:bg-neutral-100 dark:bg-neutral-800/60 dark:hover:bg-neutral-800 rounded-lg transition-colors cursor-pointer"
                  >
                    {hook.is_active ? 'Disable' : 'Enable'}
                  </button>

                  <button
                    onClick={() => {
                      if (confirm(`Permanently delete webhook "${hook.name}"?`)) {
                        deleteMutation.mutate(hook.id);
                      }
                    }}
                    title="Delete webhook"
                    className="p-1.5 text-neutral-400 hover:text-red-600 dark:hover:text-red-400 hover:bg-red-50 dark:hover:bg-red-950/40 rounded-lg transition-colors cursor-pointer"
                  >
                    <Trash2 className="w-4 h-4" />
                  </button>
                </div>
              </div>

              {testStatus && testStatus.id === hook.id && (
                <div
                  className={`mt-3 p-2.5 rounded-lg text-xs flex items-center gap-2 ${
                    testStatus.ok
                      ? 'bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border border-emerald-500/20'
                      : 'bg-red-50 dark:bg-red-950/40 text-red-800 dark:text-red-300 border border-red-200 dark:border-red-800/50'
                  }`}
                >
                  <CheckCircle2 className="w-4 h-4 shrink-0" />
                  <span>{testStatus.message}</span>
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      {/* Modals */}
      {activeLogsWebhook && (
        <WebhookLogsModal
          webhook={activeLogsWebhook}
          roomName={getRoomName(activeLogsWebhook.default_room_id)}
          logsData={logsData}
          isLoading={isLoadingLogs}
          onRefresh={refetchLogs}
          onClose={() => setActiveLogsWebhook(null)}
        />
      )}

      {isCreateOpen && (
        <CreateWebhookModal
          rooms={rooms}
          isPending={createMutation.isPending}
          formError={formError}
          onClose={() => setIsCreateOpen(false)}
          onSubmit={(body) => createMutation.mutate(body)}
          setFormError={setFormError}
        />
      )}

      {secretModalData && (
        <SecretModal
          data={secretModalData}
          onClose={() => setSecretModalData(null)}
        />
      )}
    </div>
  );
};

export const webhooksModule: FrontendModule = {
  id: 'webhooks',
  name: 'Webhooks',
  description: 'Inbound message and alert webhook gateway',
  iconName: 'Webhook',
  navTab: {
    label: 'Webhooks',
  },
  MainView: WebhooksView,
  handleSseEvent: (event, queryClient) => {
    if (
      event.type === 'WebhookCreated' ||
      event.type === 'WebhookUpdated' ||
      event.type === 'WebhookDeleted' ||
      event.type === 'WebhookTriggered'
    ) {
      queryClient.invalidateQueries({ queryKey: ['webhooks'] });
      queryClient.invalidateQueries({ queryKey: ['webhook-logs'] });
    }
  },
};
