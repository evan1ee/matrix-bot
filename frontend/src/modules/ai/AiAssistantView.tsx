import React, { useState, useEffect, useRef } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import type { SystemEvent } from '../../core/types';
import { Settings, Trash2, Calendar, Plus, X } from 'lucide-react';
import type { AiConversation, AiMessage, AiUserConfig } from './types';
import { AiSettingsModal } from './SettingsModal';
import { RetrospectiveModal } from './RetrospectiveModal';
import { useSettings, browserTimezone } from '../../core/settings';
import { registerAiPromptListener } from './aiPromptBus';
import { useAiTurn } from './useAiTurn';
import { getSnapshot } from './liveTurn';
import { MessageList } from './MessageList';
import { Composer } from './Composer';
import { ConversationSwitcher } from './ConversationSwitcher';
import { showToast } from '../../core/toast';

const MAX_PROMPT_CHARS = 4000;

export const AiAssistantView: React.FC<{
  events?: SystemEvent[];
  isConnected?: boolean;
  autoFocus?: boolean;
}> = ({ autoFocus }) => {
  const queryClient = useQueryClient();
  const { timezone } = useSettings();
  const effectiveTimezone = timezone && timezone !== 'UTC' ? timezone : browserTimezone();
  const [prompt, setPrompt] = useState('');

  const cachedConvs = queryClient.getQueryData<AiConversation[]>(['ai-conversations']);
  const { turn, startTurn, stopTurn, clearTurn } = useAiTurn();
  const turnActive = turn?.phase === 'running' || turn?.phase === 'stopping';
  const [activeConversationId, setActiveConversationId] = useState<string | null>(
    () => turn?.conversationId ?? (cachedConvs && cachedConvs.length > 0 ? cachedConvs[0].id : null)
  );

  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [isRetroOpen, setIsRetroOpen] = useState(false);
  /** The stored message an edited prompt will replace; it and everything after it go on send. */
  const [editingId, setEditingId] = useState<string | null>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (autoFocus) inputRef.current?.focus();
  }, [autoFocus]);

  // A failed turn's notice has been seen once the user leaves; do not bring it back on return.
  useEffect(
    () => () => {
      if (getSnapshot()?.phase === 'error') clearTurn();
    },
    [clearTurn]
  );

  const { data: config } = useQuery({
    queryKey: ['ai-config'],
    queryFn: () => fetchApi<AiUserConfig>('/account/ai-config'),
  });

  const { data: conversations = [] } = useQuery({
    queryKey: ['ai-conversations'],
    queryFn: () => fetchApi<AiConversation[]>('/ai/conversations'),
  });

  const { data: roomsData } = useQuery<{ room_id: string; name: string; is_direct?: boolean }[]>({
    queryKey: ['rooms'],
    queryFn: () => fetchApi('/rooms'),
  });

  const defaultRoom = roomsData?.find((r) => r.is_direct)?.room_id || roomsData?.[0]?.room_id;

  const effectiveConvId =
    activeConversationId || (conversations.length > 0 ? conversations[0].id : null);

  // The turn's conversation becomes the active one as soon as the server names it.
  const liveConversationId = turn?.conversationId ?? null;
  useEffect(() => {
    if (liveConversationId) setActiveConversationId(liveConversationId);
  }, [liveConversationId]);

  // A turn that fails puts its prompt back, unless the user already typed something new.
  // Sending again replaces the error turn, so the notice goes with the next attempt.
  const failedPrompt = turn?.phase === 'error' ? turn.prompt : null;
  useEffect(() => {
    if (failedPrompt) setPrompt((prev) => (prev ? prev : failedPrompt));
  }, [failedPrompt]);

  const { data: messages = [], isLoading: isLoadingMessages } = useQuery({
    queryKey: ['ai-messages', effectiveConvId],
    queryFn: () =>
      effectiveConvId
        ? fetchApi<AiMessage[]>(`/ai/conversations/${effectiveConvId}/messages`)
        : Promise.resolve([]),
    enabled: !!effectiveConvId,
  });

  useEffect(() => {
    if (conversations.length === 0) {
      // The list can be stale right after a turn created its conversation, so
      // an empty list alone does not drop the active one: its history must be
      // empty too (the conversation was deleted elsewhere).
      if (activeConversationId !== null && !turn && !isLoadingMessages && messages.length === 0) {
        setActiveConversationId(null);
      }
    } else if (!activeConversationId) {
      setActiveConversationId(conversations[0].id);
    }
  }, [conversations, activeConversationId, turn, isLoadingMessages, messages.length]);

  /** The one path into a turn. A tab has one live turn; a prompt that arrives meanwhile is not sent. */
  const runPrompt = (text: string) => {
    const trimmed = text.trim();
    if (!trimmed || trimmed.length > MAX_PROMPT_CHARS) return;
    if (turnActive) {
      // Park it in the input for later, but never over something the user typed.
      setPrompt((prev) => (prev ? prev : trimmed));
      return;
    }
    if (editingId) {
      void sendEdited(trimmed);
      return;
    }
    if (startTurn(trimmed, effectiveConvId, defaultRoom)) {
      setPrompt((prev) => (prev.trim() === trimmed ? '' : prev));
    }
  };

  /** Edit & Resend: drop the old message and what followed it, then send the new text from there. */
  const sendEdited = async (text: string) => {
    const target = editingId;
    if (!target || !effectiveConvId) return;
    try {
      await fetchApi<void>(`/ai/conversations/${effectiveConvId}/truncate`, {
        method: 'POST',
        body: JSON.stringify({ from_message_id: target }),
      });
    } catch (err) {
      showToast({
        title: 'Could not edit message',
        description: err instanceof Error ? err.message : 'Request failed',
        type: 'error',
      });
      return;
    }
    // The live turn's baseline is read from this cache, so cut it before sending.
    queryClient.setQueryData<AiMessage[]>(['ai-messages', effectiveConvId], (old) => {
      const index = old ? old.findIndex((m) => m.id === target) : -1;
      return old && index >= 0 ? old.slice(0, index) : old;
    });
    setEditingId(null);
    if (startTurn(text, effectiveConvId, defaultRoom)) setPrompt('');
  };

  const beginEdit = (m: AiMessage) => {
    setEditingId(m.id);
    setPrompt(m.content);
    setTimeout(() => inputRef.current?.focus(), 50);
  };

  const cancelEdit = () => {
    setEditingId(null);
    setPrompt('');
  };

  // Prompts from other modules arrive through a bus and a window event. The
  // listeners register once and call the latest `runPrompt` through a ref.
  const runPromptRef = useRef(runPrompt);
  runPromptRef.current = runPrompt;

  useEffect(() => {
    const handle = (text: string, execute: boolean) => {
      const trimmed = text.trim();
      if (!trimmed || trimmed.length > MAX_PROMPT_CHARS) return;
      if (execute) {
        runPromptRef.current(trimmed);
      } else {
        // Never overwrite what the user is typing.
        setPrompt((prev) => (prev ? prev : trimmed));
        setTimeout(() => inputRef.current?.focus(), 50);
      }
    };
    const unregister = registerAiPromptListener(handle);

    const handleCustomEvent = (e: Event) => {
      const detail = (e as CustomEvent<{ prompt?: string; execute?: boolean }>).detail;
      if (detail?.prompt) {
        handle(detail.prompt, detail.execute !== false);
      } else {
        setTimeout(() => inputRef.current?.focus(), 50);
      }
    };
    window.addEventListener('app:ai-prompt', handleCustomEvent);

    return () => {
      unregister();
      window.removeEventListener('app:ai-prompt', handleCustomEvent);
    };
  }, []);

  const deleteMutation = useMutation({
    mutationFn: (convId: string) => fetchApi<void>(`/ai/conversations/${convId}`, { method: 'DELETE' }),
    onSuccess: (_data, convId) => {
      queryClient.setQueryData<AiConversation[]>(['ai-conversations'], (old = []) =>
        old.filter((c) => c.id !== convId)
      );
      setActiveConversationId(null);
      setEditingId(null);
      queryClient.invalidateQueries({ queryKey: ['ai-conversations'] });
      queryClient.invalidateQueries({ queryKey: ['ai-messages'] });
    },
  });

  // "New chat" mints an empty thread: a turn without a conversation id would
  // resume the room's latest one, and the old thread must stay in the list.
  const newChatMutation = useMutation({
    mutationFn: () =>
      fetchApi<AiConversation>('/ai/conversations', {
        method: 'POST',
        body: JSON.stringify({ room_id: defaultRoom ?? null }),
      }),
    onSuccess: (conv) => {
      queryClient.setQueryData<AiConversation[]>(['ai-conversations'], (old = []) => [
        conv,
        ...old.filter((c) => c.id !== conv.id),
      ]);
      queryClient.setQueryData<AiMessage[]>(['ai-messages', conv.id], []);
      setActiveConversationId(conv.id);
      setEditingId(null);
      setPrompt('');
      setTimeout(() => inputRef.current?.focus(), 50);
    },
  });

  const renameMutation = useMutation({
    mutationFn: ({ id, title }: { id: string; title: string }) =>
      fetchApi<AiConversation>(`/ai/conversations/${id}/title`, {
        method: 'PATCH',
        body: JSON.stringify({ title }),
      }),
    onSuccess: (conv) => {
      queryClient.setQueryData<AiConversation[]>(['ai-conversations'], (old = []) =>
        old.map((c) => (c.id === conv.id ? conv : c))
      );
    },
    onError: (err) =>
      showToast({
        title: 'Could not rename conversation',
        description: err instanceof Error ? err.message : 'Request failed',
        type: 'error',
      }),
  });

  const startNewChat = () => {
    // An empty thread is already a new chat; do not pile up blanks.
    if (effectiveConvId && messages.length === 0 && !turn) {
      inputRef.current?.focus();
      return;
    }
    newChatMutation.mutate();
  };

  const deleteActive = () => {
    if (!effectiveConvId) return;
    if (window.confirm('Delete this conversation and its messages? This cannot be undone.')) {
      deleteMutation.mutate(effectiveConvId);
    }
  };

  const saveConfigMutation = useMutation({
    mutationFn: (input: Partial<AiUserConfig>) =>
      fetchApi<AiUserConfig>('/account/ai-config', { method: 'PUT', body: JSON.stringify(input) }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['ai-config'] });
      setIsSettingsOpen(false);
    },
  });

  return (
    <div className="flex-1 flex flex-col min-h-0 space-y-4">
      {/* Top Toolbar */}
      <div className="flex items-center justify-between gap-3 border-b border-neutral-200 dark:border-neutral-800 pb-3">
        <div className="flex items-center gap-2">
          <span className="text-xs font-medium text-neutral-500 dark:text-neutral-400">Model</span>
          <span className="font-mono text-xs font-medium px-2 py-0.5 rounded bg-neutral-100 dark:bg-neutral-800 text-neutral-700 dark:text-neutral-300 border border-neutral-200 dark:border-neutral-700">
            {config?.model_name || 'Configured model'}
          </span>
          <ConversationSwitcher
            conversations={conversations}
            activeId={effectiveConvId}
            onSelect={(id) => {
              setActiveConversationId(id);
              setEditingId(null);
            }}
            onRename={(id, title) => renameMutation.mutate({ id, title })}
            disabled={turnActive}
          />
        </div>

        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => setIsRetroOpen(true)}
            title="Weekly Executive Retrospective"
            className="h-8 inline-flex items-center gap-1.5 px-3 text-xs font-medium rounded-md border border-neutral-200 dark:border-neutral-800 hover:bg-neutral-100 dark:hover:bg-neutral-900 text-neutral-700 dark:text-neutral-300 transition cursor-pointer shadow-2xs"
          >
            <Calendar className="w-3.5 h-3.5 text-neutral-500" />
            <span>Weekly Retro</span>
          </button>

          <button
            type="button"
            onClick={startNewChat}
            disabled={newChatMutation.isPending || turnActive}
            title="Start a new conversation"
            className="h-8 inline-flex items-center gap-1.5 px-3 text-xs font-medium rounded-md border border-neutral-200 dark:border-neutral-800 hover:bg-neutral-100 dark:hover:bg-neutral-900 text-neutral-700 dark:text-neutral-300 transition-colors duration-150 cursor-pointer disabled:opacity-50 shadow-2xs"
          >
            <Plus className="w-3.5 h-3.5 text-neutral-400" />
            <span>New Chat</span>
          </button>

          {effectiveConvId && (
            <button
              type="button"
              onClick={deleteActive}
              disabled={deleteMutation.isPending || turnActive}
              aria-label="Delete conversation"
              title="Delete this conversation"
              className="h-8 w-8 flex items-center justify-center rounded-md border border-neutral-200 dark:border-neutral-800 hover:bg-neutral-100 dark:hover:bg-neutral-900 text-neutral-400 transition-colors duration-150 cursor-pointer disabled:opacity-50 shadow-2xs"
            >
              <Trash2 className="w-3.5 h-3.5" />
            </button>
          )}

          <button
            type="button"
            onClick={() => setIsSettingsOpen(true)}
            title="Configure AI model & API keys"
            className="h-8 w-8 flex items-center justify-center text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-100 rounded-md border border-neutral-200 dark:border-neutral-800 hover:bg-neutral-100 dark:hover:bg-neutral-900 transition cursor-pointer shadow-2xs"
          >
            <Settings className="w-4 h-4" />
          </button>
        </div>
      </div>

      <MessageList
        messages={messages}
        isLoading={isLoadingMessages}
        conversationId={effectiveConvId}
        turn={turn}
        timezone={effectiveTimezone}
        onSuggestion={runPrompt}
        canEdit={!turnActive}
        onEdit={beginEdit}
      />

      {editingId && (
        <div className="flex items-center justify-between gap-2 px-3 py-1.5 rounded-md border border-neutral-200 dark:border-neutral-800 text-xs text-neutral-600 dark:text-neutral-400">
          <span>Editing an earlier message: it and the replies after it are replaced when you send.</span>
          <button
            type="button"
            onClick={cancelEdit}
            aria-label="Cancel edit"
            className="inline-flex items-center gap-1 hover:text-neutral-900 dark:hover:text-neutral-100 transition-colors duration-150 cursor-pointer"
          >
            <X className="w-3 h-3" />
            Cancel
          </button>
        </div>
      )}

      <Composer
        prompt={prompt}
        onPromptChange={setPrompt}
        onSend={() => runPrompt(prompt)}
        onStop={stopTurn}
        phase={turn?.phase ?? null}
        inputRef={inputRef}
      />

      <AiSettingsModal
        isOpen={isSettingsOpen}
        onClose={() => setIsSettingsOpen(false)}
        config={config}
        onSave={(input) => saveConfigMutation.mutate(input)}
        isSaving={saveConfigMutation.isPending}
        error={saveConfigMutation.error ? (saveConfigMutation.error as Error).message : null}
      />

      <RetrospectiveModal
        isOpen={isRetroOpen}
        onClose={() => setIsRetroOpen(false)}
        onShareToChat={runPrompt}
      />
    </div>
  );
};
