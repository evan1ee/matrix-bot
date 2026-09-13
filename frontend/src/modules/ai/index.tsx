import React, { useState, useEffect, useLayoutEffect, useRef } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import { FrontendModule, SystemEvent } from '../../core/types';
import {
  Sparkles, Send, RefreshCw, Settings, Trash2, Bot, User,
  Wrench, AlertCircle, Clock, ChevronRight, Calendar
} from 'lucide-react';
import { AiConversation, AiMessage, AiChatResponse, AiUserConfig } from './types';
import { AiSettingsModal } from './SettingsModal';
import { RetrospectiveModal } from './RetrospectiveModal';
import { MarkdownRenderer } from '../../components/MarkdownRenderer';
import { useSettings, formatShortTimeInZone, browserTimezone } from '../../core/settings';

export * from './types';

let pendingAiPrompt: { prompt: string; execute: boolean } | null = null;
let promptListener: ((prompt: string, execute: boolean) => void) | null = null;

export function triggerAiPrompt(prompt: string, execute = true) {
  if (promptListener) {
    promptListener(prompt, execute);
  } else {
    pendingAiPrompt = { prompt, execute };
  }
  window.dispatchEvent(new CustomEvent('app:navigate', { detail: { moduleId: 'ai' } }));
}

const SUGGESTIONS = [
  'Spent $16.50 on lunch at Shake Shack, check off my workout habit, and remind me to call mom at 7pm',
  'Log my mood as amazing today, spent $45 on groceries, and add task Buy birthday gift',
  'Generate weekly retrospective',
  'What tasks do I have due today?',
  'Show a breakdown of my portfolio and cash balance',
  'Summarize my upcoming reminders and bills',
];

const AiAssistantView: React.FC<{
  events?: SystemEvent[];
  isConnected?: boolean;
  autoFocus?: boolean;
}> = ({ autoFocus }) => {
  const queryClient = useQueryClient();
  const { timezone } = useSettings();
  const effectiveTimezone = timezone && timezone !== 'UTC' ? timezone : browserTimezone();
  const [prompt, setPrompt] = useState('');

  const cachedConvs = queryClient.getQueryData<AiConversation[]>(['ai-conversations']);
  const [activeConversationId, setActiveConversationId] = useState<string | null>(
    () => (cachedConvs && cachedConvs.length > 0 ? cachedConvs[0].id : null)
  );

  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [isRetroOpen, setIsRetroOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const prevMessagesLengthRef = useRef(0);
  const prevConvIdRef = useRef<string | null>(null);

  useEffect(() => {
    if (autoFocus) {
      inputRef.current?.focus();
    }
  }, [autoFocus]);

  // Queries
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

  // Mutations
  const chatMutation = useMutation({
    mutationFn: (text: string) =>
      fetchApi<AiChatResponse>('/ai/chat', {
        method: 'POST',
        timeoutMs: 90_000,
        body: JSON.stringify({
          prompt: text,
          conversation_id: effectiveConvId || undefined,
          room_id: defaultRoom || undefined,
        }),
      }),
    onSuccess: (data, variables) => {
      setActiveConversationId(data.conversation_id);
      queryClient.setQueryData<AiConversation[]>(['ai-conversations'], (old = []) => {
        const existing = old.find((c) => c.id === data.conversation_id);
        const updated: AiConversation = existing
          ? { ...existing, updated_at: new Date().toISOString() }
          : {
              id: data.conversation_id,
              owner_id: '',
              room_id: defaultRoom || '',
              created_at: new Date().toISOString(),
              updated_at: new Date().toISOString(),
            };
        return [updated, ...old.filter((c) => c.id !== data.conversation_id)];
      });
      queryClient.setQueryData<AiMessage[]>(['ai-messages', data.conversation_id], (old = []) => {
        if (old.some((m) => m.role === 'user' && m.content === variables)) return old;
        return [
          ...old,
          {
            id: `temp_user_${Date.now()}`,
            conversation_id: data.conversation_id,
            role: 'user',
            content: variables,
            created_at: new Date().toISOString(),
          },
          {
            id: `temp_assistant_${Date.now()}`,
            conversation_id: data.conversation_id,
            role: 'assistant',
            content: data.message,
            created_at: new Date().toISOString(),
          },
        ];
      });
      queryClient.invalidateQueries({ queryKey: ['ai-conversations'] });
      queryClient.invalidateQueries({ queryKey: ['ai-messages', data.conversation_id] });
      setPrompt('');
    },
    onError: (_error, variables) => {
      setPrompt((prev) => (prev ? prev : variables));
    },
  });

  useEffect(() => {
    if (conversations.length === 0) {
      if (activeConversationId !== null && !chatMutation.isPending) {
        setActiveConversationId(null);
      }
    } else if (!activeConversationId) {
      setActiveConversationId(conversations[0].id);
    }
  }, [conversations, activeConversationId, chatMutation.isPending]);

  const { data: messages = [], isLoading: isLoadingMessages } = useQuery({
    queryKey: ['ai-messages', effectiveConvId],
    queryFn: () =>
      effectiveConvId
        ? fetchApi<AiMessage[]>(`/ai/conversations/${effectiveConvId}/messages`)
        : Promise.resolve([]),
    enabled: !!effectiveConvId,
  });

  // Instantly jump to the bottom when messages load or conversation changes,
  // preventing the 1-second smooth scroll animation through the whole history.
  useLayoutEffect(() => {
    const el = containerRef.current;
    if (!el || messages.length === 0) return;

    let raf: number | null = null;
    const isDifferentConv = prevConvIdRef.current !== effectiveConvId;
    const isInitialLoad = prevMessagesLengthRef.current === 0 && messages.length > 0;

    if (isDifferentConv || isInitialLoad) {
      el.scrollTop = el.scrollHeight;
      raf = requestAnimationFrame(() => {
        if (containerRef.current) {
          containerRef.current.scrollTop = containerRef.current.scrollHeight;
        }
      });
    } else if (messages.length > prevMessagesLengthRef.current) {
      el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' });
    }

    prevMessagesLengthRef.current = messages.length;
    prevConvIdRef.current = effectiveConvId;

    return () => {
      if (raf !== null) cancelAnimationFrame(raf);
    };
  }, [messages, effectiveConvId]);

  useEffect(() => {
    if (chatMutation.isPending && containerRef.current) {
      containerRef.current.scrollTo({
        top: containerRef.current.scrollHeight,
        behavior: 'smooth',
      });
    }
  }, [chatMutation.isPending]);

  useEffect(() => {
    promptListener = (text: string, execute: boolean) => {
      const trimmed = text.trim();
      if (!trimmed || trimmed.length > 4000) return;
      if (execute) {
        setPrompt('');
        chatMutation.mutate(trimmed);
      } else {
        setPrompt(trimmed);
        setTimeout(() => inputRef.current?.focus(), 50);
      }
    };

    if (pendingAiPrompt) {
      const { prompt: text, execute } = pendingAiPrompt;
      pendingAiPrompt = null;
      const trimmed = text.trim();
      if (trimmed && trimmed.length <= 4000) {
        if (execute) {
          setPrompt('');
          chatMutation.mutate(trimmed);
        } else {
          setPrompt(trimmed);
          setTimeout(() => inputRef.current?.focus(), 50);
        }
      }
    }

    const handleCustomEvent = (e: Event) => {
      const detail = (e as CustomEvent<{ prompt?: string; execute?: boolean }>).detail;
      const text = detail?.prompt;
      if (text) {
        const trimmed = text.trim();
        if (!trimmed || trimmed.length > 4000) return;
        if (detail.execute !== false) {
          setPrompt('');
          chatMutation.mutate(trimmed);
        } else {
          setPrompt(trimmed);
          setTimeout(() => inputRef.current?.focus(), 50);
        }
      } else {
        setTimeout(() => inputRef.current?.focus(), 50);
      }
    };
    window.addEventListener('app:ai-prompt', handleCustomEvent);

    return () => {
      promptListener = null;
      window.removeEventListener('app:ai-prompt', handleCustomEvent);
    };
  }, [chatMutation]);

  const resetMutation = useMutation({
    mutationFn: (convId: string) =>
      fetchApi<void>(`/ai/conversations/${convId}`, {
        method: 'DELETE',
      }),
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
      fetchApi<AiUserConfig>('/account/ai-config', {
        method: 'PUT',
        body: JSON.stringify(input),
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['ai-config'] });
      setIsSettingsOpen(false);
    },
  });

  const sendChat = (text: string) => {
    const trimmed = text.trim();
    if (!trimmed || trimmed.length > 4000 || chatMutation.isPending) return;
    setPrompt('');
    chatMutation.mutate(trimmed);
  };

  const handleSend = (e?: React.FormEvent) => {
    if (e) e.preventDefault();
    sendChat(prompt);
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

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

      {/* Messages Thread Container */}
      <div
        ref={containerRef}
        className="flex-1 overflow-y-auto rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/40 dark:bg-neutral-900/30 p-4 sm:p-6 space-y-5"
      >
        {isLoadingMessages && messages.length === 0 && !chatMutation.isPending ? (
          <div className="py-20 text-center text-xs font-mono text-neutral-500 dark:text-neutral-400">Loading conversation...</div>
        ) : messages.length === 0 && !chatMutation.isPending ? (
          <div className="py-16 text-center max-w-lg mx-auto space-y-6">
            <div className="w-10 h-10 rounded-full bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400 flex items-center justify-center mx-auto">
              <Bot className="w-5 h-5" />
            </div>
            <div>
              <h3 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100 font-sans">How can I help you today?</h3>
              <p className="text-xs text-neutral-500 dark:text-neutral-400 mt-1 font-sans">
                Ask questions, execute multi-action quick capture, or manage tasks, reminders, bills, and portfolios.
              </p>
            </div>

            <div className="grid grid-cols-1 sm:grid-cols-2 gap-2 text-left pt-2">
              {SUGGESTIONS.map((s, idx) => (
                <button
                  key={idx}
                  type="button"
                  onClick={() => sendChat(s)}
                  className="p-3 rounded-md border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 hover:border-neutral-400 dark:hover:border-neutral-700 text-xs text-neutral-800 dark:text-neutral-200 transition flex items-center justify-between group cursor-pointer shadow-2xs font-sans"
                >
                  <span className="line-clamp-2">{s}</span>
                  <ChevronRight className="w-3.5 h-3.5 text-neutral-400 group-hover:text-neutral-900 dark:group-hover:text-neutral-100 shrink-0 ml-1.5 transition" />
                </button>
              ))}
            </div>
          </div>
        ) : (
          messages
            .filter((m) => m.role !== 'system' && (m.role !== 'assistant' || m.content.trim().length > 0))
            .map((m) => (
              <div
                key={m.id}
                className={`flex gap-3 max-w-3xl ${
                  m.role === 'user' ? 'ml-auto justify-end' : 'mr-auto justify-start'
                }`}
              >
                {m.role !== 'user' && (
                  <div className="w-7 h-7 rounded-full bg-neutral-100 dark:bg-neutral-800 text-neutral-700 dark:text-neutral-300 flex items-center justify-center shrink-0 mt-0.5">
                    <Bot className="w-3.5 h-3.5" />
                  </div>
                )}

                <div
                  className={`p-3.5 rounded-lg text-sm leading-relaxed space-y-2 max-w-2xl font-sans ${
                    m.role === 'user'
                      ? 'bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900 shadow-xs'
                      : m.role === 'tool'
                      ? 'bg-neutral-100/80 dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-neutral-800 dark:text-neutral-200 font-mono text-xs'
                      : 'bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-neutral-900 dark:text-neutral-100 shadow-xs'
                  }`}
                >
                  {m.role === 'tool' ? (
                    <div className="flex items-center gap-1.5 text-xs text-neutral-600 dark:text-neutral-400 font-sans font-medium">
                      <Wrench className="w-3.5 h-3.5" />
                      <span>Tool Result:</span>
                      <code className="font-mono text-[11px] px-1 py-0.5 rounded bg-neutral-200/60 dark:bg-neutral-800 text-neutral-700 dark:text-neutral-300">
                        {m.tool_name || 'internal_tool'}
                      </code>
                    </div>
                  ) : null}

                  {m.role === 'tool' ? (
                    <div className="max-h-40 overflow-y-auto rounded bg-neutral-200/40 dark:bg-neutral-950/50 p-2 font-mono text-[11px] leading-relaxed whitespace-pre-wrap break-all border border-neutral-200/60 dark:border-neutral-800/60">
                      {m.content}
                    </div>
                  ) : (
                    <MarkdownRenderer content={m.content} />
                  )}

                  <div
                    className={`text-[10px] font-mono flex items-center gap-1 pt-1 ${
                      m.role === 'user' ? 'text-neutral-300 dark:text-neutral-600 justify-end' : 'text-neutral-400 dark:text-neutral-500'
                    }`}
                  >
                    <Clock className="w-2.5 h-2.5" />
                    <span>{formatShortTimeInZone(m.created_at, effectiveTimezone)}</span>
                  </div>
                </div>

                {m.role === 'user' && (
                  <div className="w-7 h-7 rounded-full bg-neutral-200 dark:bg-neutral-800 text-neutral-700 dark:text-neutral-300 flex items-center justify-center shrink-0 mt-0.5 font-mono text-[11px]">
                    <User className="w-3.5 h-3.5" />
                  </div>
                )}
              </div>
            ))
        )}

        {chatMutation.isPending && chatMutation.variables && !messages.some((m) => m.role === 'user' && m.content === chatMutation.variables) && (
          <div className="flex gap-3 max-w-3xl ml-auto justify-end animate-in fade-in-0">
            <div className="p-3.5 rounded-lg text-sm leading-relaxed space-y-2 max-w-2xl font-sans bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900 shadow-xs">
              <MarkdownRenderer content={chatMutation.variables} />
              <div className="text-[10px] font-mono flex items-center gap-1 pt-1 text-neutral-300 dark:text-neutral-600 justify-end">
                <Clock className="w-2.5 h-2.5" />
                <span>{formatShortTimeInZone(new Date().toISOString(), effectiveTimezone)}</span>
              </div>
            </div>
            <div className="w-7 h-7 rounded-full bg-neutral-200 dark:bg-neutral-800 text-neutral-700 dark:text-neutral-300 flex items-center justify-center shrink-0 mt-0.5 font-mono text-[11px]">
              <User className="w-3.5 h-3.5" />
            </div>
          </div>
        )}

        {chatMutation.isPending && (
          <div className="flex gap-3 max-w-3xl mr-auto justify-start animate-in fade-in-0">
            <div className="w-7 h-7 rounded-full bg-neutral-100 dark:bg-neutral-800 text-neutral-700 dark:text-neutral-300 flex items-center justify-center shrink-0 mt-0.5">
              <Bot className="w-3.5 h-3.5" />
            </div>
            <div className="p-3.5 rounded-lg text-sm bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-neutral-900 dark:text-neutral-100 shadow-2xs flex items-center gap-2">
              <RefreshCw className="w-3.5 h-3.5 animate-spin text-neutral-500" />
              <span className="text-xs text-neutral-500 font-sans">Thinking and executing tools...</span>
            </div>
          </div>
        )}

        {chatMutation.error && (
          <div className="p-3 bg-red-500/10 border border-red-500/20 rounded-md text-red-600 dark:text-red-400 text-xs flex items-start gap-2 max-w-2xl mx-auto font-sans">
            <AlertCircle className="w-4 h-4 shrink-0 mt-0.5" />
            <div>
              <strong>Generation failed:</strong> {(chatMutation.error as Error).message}
            </div>
          </div>
        )}

        <div ref={messagesEndRef} />
      </div>

      {/* Input Area */}
      <form onSubmit={handleSend} className="relative">
        <textarea
          ref={inputRef}
          rows={2}
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="Ask AI assistant... (Enter to send, Shift+Enter for newline)"
          className="w-full px-3.5 py-2.5 pr-12 bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 rounded-lg text-sm focus:outline-none focus:border-neutral-500 font-sans resize-none shadow-2xs"
        />
        <button
          type="submit"
          disabled={!prompt.trim() || chatMutation.isPending}
          className="absolute right-2.5 bottom-2.5 p-1.5 bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 rounded hover:bg-neutral-800 dark:hover:bg-neutral-200 transition disabled:opacity-40 cursor-pointer"
        >
          <Send className="w-3.5 h-3.5" />
        </button>
      </form>

      {/* Settings Modal */}
      <AiSettingsModal
        isOpen={isSettingsOpen}
        onClose={() => setIsSettingsOpen(false)}
        config={config}
        onSave={(input) => saveConfigMutation.mutate(input)}
        isSaving={saveConfigMutation.isPending}
        error={saveConfigMutation.error ? (saveConfigMutation.error as Error).message : null}
      />

      {/* Retrospective Modal */}
      <RetrospectiveModal
        isOpen={isRetroOpen}
        onClose={() => setIsRetroOpen(false)}
        onShareToChat={(text) => sendChat(text)}
      />
    </div>
  );
};

export const aiModule: FrontendModule = {
  id: 'ai',
  name: 'AI Assistant',
  description: 'Conversational assistant with tool calling',
  iconName: 'Sparkles',
  navTab: {
    label: 'AI Assistant',
  },
  MainView: AiAssistantView,
  handleSseEvent: (event, queryClient) => {
    if (event.type === 'AiConversationUpdated' || event.type === 'AiConversationDeleted') {
      queryClient.invalidateQueries({ queryKey: ['ai-conversations'] });
      queryClient.invalidateQueries({ queryKey: ['ai-messages'] });
    }
    if (event.type === 'AiConfigUpdated') {
      queryClient.invalidateQueries({ queryKey: ['ai-config'] });
    }
  },
};
