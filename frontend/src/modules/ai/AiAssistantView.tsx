import React, { useState, useEffect, useRef } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import type { SystemEvent } from '../../core/types';
import { Settings, Trash2, Calendar } from 'lucide-react';
import type { AiConversation, AiMessage, AiUserConfig } from './types';
import { AiSettingsModal } from './SettingsModal';
import { RetrospectiveModal } from './RetrospectiveModal';
import { useSettings, browserTimezone } from '../../core/settings';
import { registerAiPromptListener } from './aiPromptBus';
import { useAiTurn } from './useAiTurn';
import { getSnapshot } from './liveTurn';
import { MessageList } from './MessageList';
import { Composer } from './Composer';

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
    if (startTurn(trimmed, effectiveConvId, defaultRoom)) {
      setPrompt((prev) => (prev.trim() === trimmed ? '' : prev));
    }
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

  const resetMutation = useMutation({
    mutationFn: (convId: string) => fetchApi<void>(`/ai/conversations/${convId}`, { method: 'DELETE' }),
    onSuccess: (_data, convId) => {
      queryClient.setQueryData<AiConversation[]>(['ai-conversations'], (old = []) =>
        old.filter((c) => c.id !== convId)
      );
      setActiveConversationId(null);
      queryClient.invalidateQueries({ queryKey: ['ai-conversations'] });
      queryClient.invalidateQueries({ queryKey: ['ai-messages'] });
    },
  });

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
            {config?.model_name || 'gpt-4o-mini'}
          </span>
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

          {effectiveConvId && (
            <button
              type="button"
              onClick={() => resetMutation.mutate(effectiveConvId)}
              disabled={resetMutation.isPending}
              title="Reset current conversation"
              className="h-8 inline-flex items-center gap-1.5 px-3 text-xs font-medium rounded-md border border-neutral-200 dark:border-neutral-800 hover:bg-neutral-100 dark:hover:bg-neutral-900 text-neutral-700 dark:text-neutral-300 transition cursor-pointer shadow-2xs"
            >
              <Trash2 className="w-3.5 h-3.5 text-neutral-400" />
              <span>New Chat</span>
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
      />

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
