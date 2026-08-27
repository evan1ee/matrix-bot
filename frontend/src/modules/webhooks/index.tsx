import React, { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import { FrontendModule, SystemEvent } from '../../core/types';
import {
  Webhook as WebhookIcon, Plus, Trash2, RefreshCw, Send,
  Copy, Check, AlertTriangle, ShieldCheck, Radio, CheckCircle2,
  Terminal, Globe, History, X, Clock, AlertCircle, FileText
} from 'lucide-react';
import { CustomSelect } from '../../components/CustomSelect';

export interface WebhookItem {
  id: string;
  owner_id: string;
  name: string;
  token_prefix: string;
  default_room_id: string;
  is_active: boolean;
  total_delivered_count: number;
  failure_count: number;
  last_triggered_at?: string | null;
  created_at: string;
}

export interface WebhookOutboxItem {
  id: string;
  webhook_id: string;
  owner_id: string;
  room_id: string;
  severity: string;
  title?: string | null;
  html_body: string;
  plain_body: string;
  status: string;
  attempts: number;
  max_attempts: number;
  next_attempt_at: string;
  last_error?: string | null;
  created_at: string;
  delivered_at?: string | null;
}

export interface WebhookLogsResponse {
  items: WebhookOutboxItem[];
  total: number;
  limit: number;
  offset: number;
}

export interface CreatedWebhookResponse {
  webhook: WebhookItem;
  raw_token: string;
  example_curl_json: string;
  example_curl_telegram: string;
}

export interface JoinedRoomInfo {
  room_id: string;
  name: string;
}

const WebhooksView: React.FC<{
  events?: SystemEvent[];
  isConnected?: boolean;
}> = () => {
  const queryClient = useQueryClient();
  const [isCreateOpen, setIsCreateOpen] = useState(false);
  const [newName, setNewName] = useState('');
  const [newRoomId, setNewRoomId] = useState('');
  const [formError, setFormError] = useState<string | null>(null);

  // Modal shown once when a token is generated/rotated
  const [secretModalData, setSecretModalData] = useState<CreatedWebhookResponse | null>(null);
  const [copiedType, setCopiedType] = useState<string | null>(null);
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

  const roomOptions = rooms.map((r) => ({
    value: r.room_id,
    label: r.name || r.room_id,
  }));

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
      setNewName('');
      setNewRoomId('');
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

  const handleCopy = (text: string, type: string) => {
    navigator.clipboard.writeText(text);
    setCopiedType(type);
    setTimeout(() => setCopiedType(null), 2000);
  };

  const handleCreateSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!newName.trim()) {
      setFormError('Webhook name is required');
      return;
    }
    const targetRoom = newRoomId || (rooms.length > 0 ? rooms[0].room_id : '');
    if (!targetRoom) {
      setFormError('Please select a target Matrix room');
      return;
    }
    createMutation.mutate({
      name: newName.trim(),
      default_room_id: targetRoom,
    });
  };

  const getSeverityBadge = (severity: string) => {
    switch (severity.toLowerCase()) {
      case 'critical':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 text-[11px] font-semibold rounded-md bg-red-500/15 text-red-600 dark:text-red-400 border border-red-500/20">
            🔴 CRITICAL
          </span>
        );
      case 'warning':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 text-[11px] font-semibold rounded-md bg-amber-500/15 text-amber-600 dark:text-amber-400 border border-amber-500/20">
            🟡 WARNING
          </span>
        );
      case 'success':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 text-[11px] font-semibold rounded-md bg-emerald-500/15 text-emerald-600 dark:text-emerald-400 border border-emerald-500/20">
            🟢 SUCCESS
          </span>
        );
      default:
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 text-[11px] font-semibold rounded-md bg-blue-500/15 text-blue-600 dark:text-blue-400 border border-blue-500/20">
            🔵 INFO
          </span>
        );
    }
  };

  const getStatusBadge = (status: string) => {
    switch (status.toLowerCase()) {
      case 'delivered':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 text-[11px] font-medium rounded-full bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border border-emerald-500/20">
            <CheckCircle2 className="w-3 h-3" /> Delivered
          </span>
        );
      case 'failed':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 text-[11px] font-medium rounded-full bg-destructive/10 text-destructive border border-destructive/20">
            <AlertCircle className="w-3 h-3" /> Failed
          </span>
        );
      default:
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 text-[11px] font-medium rounded-full bg-amber-500/10 text-amber-600 dark:text-amber-400 border border-amber-500/20">
            <Clock className="w-3 h-3 animate-spin" /> In Queue
          </span>
        );
    }
  };

  return (
    <div className="space-y-6 max-w-6xl mx-auto p-4 sm:p-6 lg:p-8">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4 border-b border-neutral-200 dark:border-neutral-800 pb-5">
        <div>
          <h1 className="text-2xl font-bold tracking-tight text-neutral-900 dark:text-neutral-100 flex items-center gap-2">
            <WebhookIcon className="w-6 h-6 text-blue-600 dark:text-blue-400" />
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
            if (!newRoomId && rooms.length > 0) {
              setNewRoomId(rooms[0].room_id);
            }
          }}
          className="inline-flex items-center gap-2 px-4 py-2 bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 font-medium text-sm rounded-lg hover:bg-neutral-800 dark:hover:bg-neutral-200 transition-colors shadow-sm cursor-pointer"
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
          <div className="w-12 h-12 rounded-full bg-blue-500/10 text-blue-600 dark:text-blue-400 flex items-center justify-center mx-auto mb-4">
            <Globe className="w-6 h-6" />
          </div>
          <h3 className="text-base font-semibold text-neutral-900 dark:text-neutral-100">No inbound webhooks configured</h3>
          <p className="text-sm text-neutral-500 dark:text-neutral-400 max-w-md mx-auto mt-1 mb-6">
            Create an inbound webhook endpoint to send server alerts, CI/CD statuses, and monitoring events directly into your Matrix rooms.
          </p>
          <button
            onClick={() => {
              setIsCreateOpen(true);
              if (!newRoomId && rooms.length > 0) {
                setNewRoomId(rooms[0].room_id);
              }
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
                      className={`inline-flex items-center gap-1 px-2 py-0.5 text-xs font-medium rounded-full ${
                        hook.is_active
                          ? 'bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border border-emerald-500/20'
                          : 'bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-300 border border-neutral-200 dark:border-neutral-700'
                      }`}
                    >
                      <Radio className={`w-3 h-3 ${hook.is_active ? 'animate-pulse text-emerald-500' : ''}`} />
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
                    <History className="w-3.5 h-3.5 text-blue-600 dark:text-blue-400" />
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

      {/* Message History & Delivery Logs Drawer Modal */}
      {activeLogsWebhook && (
        <div className="fixed inset-0 z-50 bg-black/60 dark:bg-black/80 backdrop-blur-sm flex items-center justify-center p-3 sm:p-4" role="dialog" aria-modal="true">
          <div className="bg-white dark:bg-neutral-950 text-neutral-900 dark:text-neutral-100 w-full max-w-3xl rounded-xl border border-neutral-200 dark:border-neutral-800 shadow-2xl overflow-hidden animate-in fade-in-0 zoom-in-95 max-h-[88vh] flex flex-col">
            <div className="p-4 sm:p-5 border-b border-neutral-200 dark:border-neutral-800 flex items-center justify-between gap-3 bg-neutral-50 dark:bg-neutral-900/50">
              <div className="flex items-center gap-2.5 min-w-0">
                <History className="w-5 h-5 text-blue-600 dark:text-blue-400 shrink-0" />
                <div className="min-w-0">
                  <h2 className="text-base font-bold text-neutral-900 dark:text-neutral-100 truncate">
                    Delivery Logs: {activeLogsWebhook.name}
                  </h2>
                  <p className="text-xs text-neutral-500 dark:text-neutral-400 truncate">
                    Target Room: {getRoomName(activeLogsWebhook.default_room_id)} &bull; {logsData?.total ?? 0} total events
                  </p>
                </div>
              </div>
              <div className="flex items-center gap-2">
                <button
                  onClick={() => refetchLogs()}
                  title="Refresh logs"
                  className="p-1.5 text-neutral-500 hover:text-neutral-900 dark:text-neutral-400 dark:hover:text-neutral-100 bg-neutral-100 hover:bg-neutral-200 dark:bg-neutral-800 dark:hover:bg-neutral-700 rounded-lg transition-colors cursor-pointer"
                >
                  <RefreshCw className={`w-4 h-4 ${isLoadingLogs ? 'animate-spin' : ''}`} />
                </button>
                <button
                  onClick={() => setActiveLogsWebhook(null)}
                  title="Close logs"
                  className="p-1.5 text-neutral-500 hover:text-neutral-900 dark:text-neutral-400 dark:hover:text-neutral-100 bg-neutral-100 hover:bg-neutral-200 dark:bg-neutral-800 dark:hover:bg-neutral-700 rounded-lg transition-colors cursor-pointer"
                >
                  <X className="w-4 h-4" />
                </button>
              </div>
            </div>

            <div className="p-4 sm:p-6 overflow-y-auto space-y-3.5 flex-1">
              {isLoadingLogs ? (
                <div className="py-12 text-center text-sm text-neutral-500 dark:text-neutral-400">Loading message history...</div>
              ) : !logsData || logsData.items.length === 0 ? (
                <div className="py-12 text-center text-neutral-500 dark:text-neutral-400 space-y-2">
                  <FileText className="w-8 h-8 mx-auto text-neutral-400 dark:text-neutral-600" />
                  <p className="text-sm font-medium text-neutral-900 dark:text-neutral-100">No messages received yet</p>
                  <p className="text-xs max-w-sm mx-auto">
                    Messages and alerts sent to this webhook will be saved and displayed here.
                  </p>
                </div>
              ) : (
                logsData.items.map((item) => (
                  <div
                    key={item.id}
                    className="p-4 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/80 dark:bg-neutral-900/80 space-y-2.5 shadow-2xs"
                  >
                    <div className="flex items-center justify-between gap-2 flex-wrap">
                      <div className="flex items-center gap-2 flex-wrap">
                        {getStatusBadge(item.status)}
                        {getSeverityBadge(item.severity)}
                        {item.title && (
                          <span className="font-semibold text-xs text-neutral-900 dark:text-neutral-100">{item.title}</span>
                        )}
                      </div>
                      <div className="text-[11px] text-neutral-500 dark:text-neutral-400 flex items-center gap-1">
                        <Clock className="w-3 h-3" />
                        {new Date(item.created_at).toLocaleString()}
                      </div>
                    </div>

                    <div className="p-2.5 bg-white dark:bg-neutral-950 rounded-md text-xs font-mono text-neutral-900 dark:text-neutral-100 whitespace-pre-wrap break-words border border-neutral-200 dark:border-neutral-800 max-h-48 overflow-y-auto">
                      {item.plain_body}
                    </div>

                    {item.last_error && (
                      <div className="p-2.5 rounded-md bg-red-50 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 text-red-800 dark:text-red-300 text-xs flex items-start gap-2">
                        <AlertTriangle className="w-3.5 h-3.5 shrink-0 mt-0.5 text-red-500" />
                        <div>
                          <strong>Error (Attempt {item.attempts}/{item.max_attempts}):</strong> {item.last_error}
                        </div>
                      </div>
                    )}
                  </div>
                ))
              )}
            </div>

            <div className="p-3.5 border-t border-neutral-200 dark:border-neutral-800 bg-neutral-50 dark:bg-neutral-900/50 flex justify-end">
              <button
                onClick={() => setActiveLogsWebhook(null)}
                className="px-4 py-1.5 text-xs font-medium bg-neutral-200 hover:bg-neutral-300 dark:bg-neutral-800 dark:hover:bg-neutral-700 text-neutral-800 dark:text-neutral-200 rounded-lg transition-colors cursor-pointer"
              >
                Close
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Create Modal */}
      {isCreateOpen && (
        <div className="fixed inset-0 z-50 bg-black/60 dark:bg-black/80 backdrop-blur-sm flex items-center justify-center p-4" role="dialog" aria-modal="true">
          <div className="bg-white dark:bg-neutral-950 text-neutral-900 dark:text-neutral-100 w-full max-w-lg rounded-xl border border-neutral-200 dark:border-neutral-800 shadow-2xl overflow-hidden animate-in fade-in-0 zoom-in-95">
            <div className="p-6 border-b border-neutral-200 dark:border-neutral-800">
              <h2 className="text-lg font-bold text-neutral-900 dark:text-neutral-100">Create Inbound Webhook</h2>
              <p className="text-xs text-neutral-500 dark:text-neutral-400 mt-1">
                Generates a secure endpoint for external scripts and alert integrations.
              </p>
            </div>

            <form onSubmit={handleCreateSubmit} className="p-6 space-y-4">
              {formError && (
                <div className="p-3 bg-red-50 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 rounded-lg text-red-800 dark:text-red-300 text-xs font-mono">
                  {formError}
                </div>
              )}

              <div>
                <label className="block text-xs font-medium text-neutral-700 dark:text-neutral-300 mb-1.5">
                  Webhook Name <span className="text-red-500">*</span>
                </label>
                <input
                  type="text"
                  required
                  placeholder="e.g. VPS Health Monitor, GitHub Actions, Uptime Kuma"
                  value={newName}
                  onChange={(e) => setNewName(e.target.value)}
                  className="w-full px-3 py-2 bg-neutral-50 dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-700 rounded-lg text-sm text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 focus:outline-none focus:ring-1 focus:ring-neutral-400 dark:focus:ring-neutral-500"
                />
              </div>

              <div>
                <label className="block text-xs font-medium text-neutral-700 dark:text-neutral-300 mb-1.5">
                  Target Matrix Room <span className="text-red-500">*</span>
                </label>
                <CustomSelect
                  options={roomOptions}
                  value={newRoomId || (rooms.length > 0 ? rooms[0].room_id : '')}
                  onChange={(val) => setNewRoomId(val)}
                  placeholder="Select destination room..."
                  className="w-full"
                />
                <p className="text-[11px] text-neutral-500 dark:text-neutral-400 mt-1">
                  Incoming alerts sent to this webhook will be delivered to this room by default.
                </p>
              </div>

              <div className="pt-4 border-t border-neutral-200 dark:border-neutral-800 flex justify-end gap-2.5">
                <button
                  type="button"
                  onClick={() => setIsCreateOpen(false)}
                  className="px-4 py-2 text-sm font-medium text-neutral-700 dark:text-neutral-300 hover:text-neutral-900 dark:hover:text-neutral-100 bg-neutral-100 dark:bg-neutral-850 hover:bg-neutral-200 dark:hover:bg-neutral-800 rounded-lg transition-colors cursor-pointer"
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  disabled={createMutation.isPending}
                  className="px-4 py-2 text-sm font-medium bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 hover:bg-neutral-800 dark:hover:bg-neutral-200 rounded-lg transition-colors disabled:opacity-50 cursor-pointer"
                >
                  {createMutation.isPending ? 'Generating...' : 'Generate Webhook'}
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* Secret Token Display Modal (Displayed once) */}
      {secretModalData && (
        <div className="fixed inset-0 z-50 bg-black/60 dark:bg-black/80 backdrop-blur-sm flex items-center justify-center p-4" role="dialog" aria-modal="true">
          <div className="bg-white dark:bg-neutral-950 text-neutral-900 dark:text-neutral-100 w-full max-w-2xl rounded-xl border border-neutral-200 dark:border-neutral-800 shadow-2xl overflow-hidden animate-in fade-in-0 zoom-in-95 max-h-[90vh] flex flex-col">
            <div className="p-6 border-b border-neutral-200 dark:border-neutral-800 bg-emerald-500/5">
              <div className="flex items-center gap-2.5 text-emerald-600 dark:text-emerald-400 font-semibold text-base">
                <ShieldCheck className="w-5 h-5" />
                Webhook Secret Generated
              </div>
              <p className="text-xs text-neutral-500 dark:text-neutral-400 mt-1">
                Your webhook <strong>{secretModalData.webhook.name}</strong> is ready.
              </p>
            </div>

            <div className="p-6 overflow-y-auto space-y-5">
              <div className="p-3.5 bg-amber-500/10 border border-amber-500/20 rounded-lg flex items-start gap-2.5 text-amber-700 dark:text-amber-400 text-xs">
                <AlertTriangle className="w-4 h-4 shrink-0 mt-0.5" />
                <div>
                  <strong>Copy this secret token now.</strong> It is stored securely as a SHA-256 hash and cannot be viewed again once this dialog is closed.
                </div>
              </div>

              <div>
                <label className="block text-xs font-semibold text-neutral-700 dark:text-neutral-300 mb-1.5 uppercase tracking-wider">
                  Secret Token
                </label>
                <div className="flex items-center gap-2">
                  <input
                    type="text"
                    readOnly
                    value={secretModalData.raw_token}
                    className="w-full font-mono text-xs px-3 py-2.5 bg-neutral-100 dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-700 rounded-lg text-neutral-900 dark:text-neutral-100 select-all"
                  />
                  <button
                    onClick={() => handleCopy(secretModalData.raw_token, 'token')}
                    className="inline-flex items-center gap-1.5 px-3 py-2.5 text-xs font-medium bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 hover:bg-neutral-800 dark:hover:bg-neutral-200 rounded-lg transition-colors shrink-0 cursor-pointer"
                  >
                    {copiedType === 'token' ? <Check className="w-4 h-4" /> : <Copy className="w-4 h-4" />}
                    {copiedType === 'token' ? 'Copied' : 'Copy'}
                  </button>
                </div>
              </div>

              {/* Sample Commands */}
              <div className="space-y-3 pt-2">
                <div className="text-xs font-semibold text-neutral-900 dark:text-neutral-100 flex items-center gap-1.5 uppercase tracking-wider">
                  <Terminal className="w-3.5 h-3.5 text-blue-600 dark:text-blue-400" />
                  Example Integrations
                </div>

                <div className="space-y-1.5">
                  <div className="text-[11px] font-medium text-neutral-500 dark:text-neutral-400 flex justify-between">
                    <span>1. Standard JSON Webhook (Alertmanager / Cron / Python)</span>
                    <button
                      onClick={() => handleCopy(secretModalData.example_curl_json, 'json')}
                      className="text-blue-600 dark:text-blue-400 hover:underline text-[11px] cursor-pointer"
                    >
                      {copiedType === 'json' ? 'Copied' : 'Copy curl'}
                    </button>
                  </div>
                  <pre className="p-3 bg-neutral-950 text-neutral-100 rounded-lg text-xs font-mono overflow-x-auto whitespace-pre-wrap border border-neutral-800">
                    {secretModalData.example_curl_json}
                  </pre>
                </div>

                <div className="space-y-1.5 pt-1">
                  <div className="text-[11px] font-medium text-neutral-500 dark:text-neutral-400 flex justify-between">
                    <span>2. Telegram Bot API Drop-in Emulation</span>
                    <button
                      onClick={() => handleCopy(secretModalData.example_curl_telegram, 'tg')}
                      className="text-blue-600 dark:text-blue-400 hover:underline text-[11px] cursor-pointer"
                    >
                      {copiedType === 'tg' ? 'Copied' : 'Copy curl'}
                    </button>
                  </div>
                  <pre className="p-3 bg-neutral-950 text-neutral-100 rounded-lg text-xs font-mono overflow-x-auto whitespace-pre-wrap border border-neutral-800">
                    {secretModalData.example_curl_telegram}
                  </pre>
                </div>
              </div>
            </div>

            <div className="p-4 border-t border-neutral-200 dark:border-neutral-800 bg-neutral-50 dark:bg-neutral-900/50 flex justify-end">
              <button
                onClick={() => setSecretModalData(null)}
                className="px-5 py-2 text-sm font-medium bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 hover:bg-neutral-800 dark:hover:bg-neutral-200 rounded-lg transition-colors cursor-pointer"
              >
                I Have Saved My Secret
              </button>
            </div>
          </div>
        </div>
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
