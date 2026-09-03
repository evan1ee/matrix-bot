import React, { useState, useEffect, useRef } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { fetchApi } from '../../core/apiClient';
import { FrontendModule, SystemEvent } from '../../core/types';
import {
  Sparkles, Send, RefreshCw, Settings, Trash2, Bot, User,
  Wrench, AlertCircle, Clock, ChevronRight
} from 'lucide-react';
import { AiConversation, AiMessage, AiChatResponse, AiUserConfig } from './types';
import { AiSettingsModal } from './SettingsModal';

export * from './types';

const SUGGESTIONS = [
  'What tasks do I have due today?',
  'Summarize my upcoming reminders and bills',
  'Show a breakdown of my portfolio and cash balance',
  'What is my position in BTC-USD and how has it performed?',
  'How much did I spend in each category over the last 30 days?',
  'Alert me if BTC-USD rises above 75,000 USD',
  'Give me a summary of my expense ledger',
];

const AiAssistantView: React.FC<{
  events?: SystemEvent[];
  isConnected?: boolean;
  autoFocus?: boolean;
}> = ({ autoFocus }) => {
  const queryClient = useQueryClient();
  const [prompt, setPrompt] = useState('');
  const [activeConversationId, setActiveConversationId] = useState<string | null>(null);
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

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

  useEffect(() => {
    if (conversations.length === 0) {
      if (activeConversationId !== null) {
        setActiveConversationId(null);
      }
    } else if (!activeConversationId || !conversations.some((c) => c.id === activeConversationId)) {
      setActiveConversationId(conversations[0].id);
    }
  }, [conversations, activeConversationId]);

  const { data: messages = [], isLoading: isLoadingMessages } = useQuery({
    queryKey: ['ai-messages', activeConversationId],
    queryFn: () =>
      activeConversationId
        ? fetchApi<AiMessage[]>(`/ai/conversations/${activeConversationId}/messages`)
        : Promise.resolve([]),
    enabled: !!activeConversationId,
  });

  const scrollToBottom = () => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  };

  useEffect(() => {
    scrollToBottom();
  }, [messages]);

  // Mutations
  const chatMutation = useMutation({
    mutationFn: (text: string) =>
      fetchApi<AiChatResponse>('/ai/chat', {
        method: 'POST',
        body: JSON.stringify({
          prompt: text,
          conversation_id: activeConversationId || undefined,
          room_id: 'web_chat',
        }),
      }),
    onSuccess: (data) => {
      setActiveConversationId(data.conversation_id);
      queryClient.invalidateQueries({ queryKey: ['ai-conversations'] });
      queryClient.invalidateQueries({ queryKey: ['ai-messages', data.conversation_id] });
      setPrompt('');
    },
  });

  const resetMutation = useMutation({
    mutationFn: (convId: string) =>
      fetchApi<void>(`/ai/conversations/${convId}`, {
        method: 'DELETE',
      }),
    onSuccess: () => {
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

  const handleSend = (e?: React.FormEvent) => {
    if (e) e.preventDefault();
    const trimmed = prompt.trim();
    if (!trimmed || trimmed.length > 4000 || chatMutation.isPending) return;
    chatMutation.mutate(trimmed);
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  return (
    <div className="flex flex-col h-[calc(100vh-4rem)] max-w-5xl mx-auto p-3 sm:p-6 space-y-4">
      {/* Header */}
      <div className="flex items-center justify-between gap-3 border-b border-neutral-200 dark:border-neutral-800 pb-4">
        <div className="flex items-center gap-2.5">
          <div className="w-9 h-9 rounded-lg bg-blue-500/10 text-blue-600 dark:text-blue-400 flex items-center justify-center">
            <Sparkles className="w-5 h-5" />
          </div>
          <div>
            <h1 className="text-xl font-bold tracking-tight text-neutral-900 dark:text-neutral-100 flex items-center gap-2">
              AI Assistant
              <span className="font-mono text-[11px] font-normal px-2 py-0.5 rounded-full bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-300 border border-neutral-200 dark:border-neutral-700">
                {config?.model_name || 'llama3.2'}
              </span>
            </h1>
            <p className="text-xs text-neutral-500 dark:text-neutral-400">
              Conversational assistant with access to your tasks, reminders, bills, portfolio, and finances.
            </p>
          </div>
        </div>

        <div className="flex items-center gap-2">
          {activeConversationId && (
            <button
              onClick={() => resetMutation.mutate(activeConversationId)}
              disabled={resetMutation.isPending}
              title="Reset current conversation"
              className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium bg-neutral-100 hover:bg-neutral-200 dark:bg-neutral-800 dark:hover:bg-neutral-700 text-neutral-700 dark:text-neutral-200 rounded-lg transition-colors cursor-pointer"
            >
              <Trash2 className="w-3.5 h-3.5 text-neutral-500 dark:text-neutral-400" />
              New Chat
            </button>
          )}

          <button
            onClick={() => setIsSettingsOpen(true)}
            title="Configure AI model & API keys"
            className="p-2 text-neutral-600 hover:text-neutral-900 dark:text-neutral-400 dark:hover:text-neutral-100 bg-neutral-100 hover:bg-neutral-200 dark:bg-neutral-800 dark:hover:bg-neutral-700 rounded-lg transition-colors cursor-pointer"
          >
            <Settings className="w-4 h-4" />
          </button>
        </div>
      </div>

      {/* Messages Thread Container */}
      <div className="flex-1 overflow-y-auto rounded-xl border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-900/40 p-4 sm:p-6 space-y-5">
        {isLoadingMessages ? (
          <div className="py-20 text-center text-sm text-neutral-500 dark:text-neutral-400">Loading conversation...</div>
        ) : messages.length === 0 ? (
          <div className="py-16 text-center max-w-lg mx-auto space-y-6">
            <div className="w-12 h-12 rounded-full bg-blue-500/10 text-blue-600 dark:text-blue-400 flex items-center justify-center mx-auto">
              <Bot className="w-6 h-6" />
            </div>
            <div>
              <h3 className="text-base font-semibold text-neutral-900 dark:text-neutral-100">How can I help you today?</h3>
              <p className="text-xs text-neutral-500 dark:text-neutral-400 mt-1">
                Ask questions, explore ideas, or ask me to manage your tasks, reminders, bills, portfolio, and financial ledger.
              </p>
            </div>

            <div className="grid grid-cols-1 sm:grid-cols-2 gap-2 text-left pt-2">
              {SUGGESTIONS.map((s, idx) => (
                <button
                  key={idx}
                  onClick={() => {
                    setPrompt(s);
                    chatMutation.mutate(s);
                  }}
                  className="p-3 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 hover:bg-neutral-50 dark:hover:bg-neutral-800/60 text-xs text-neutral-800 dark:text-neutral-200 transition-colors flex items-center justify-between group cursor-pointer shadow-2xs"
                >
                  <span className="line-clamp-2">{s}</span>
                  <ChevronRight className="w-3.5 h-3.5 text-neutral-400 group-hover:text-blue-600 dark:group-hover:text-blue-400 shrink-0 ml-1.5 transition-colors" />
                </button>
              ))}
            </div>
          </div>
        ) : (
          messages
            .filter((m) => m.role !== 'system')
            .map((m) => (
              <div
                key={m.id}
                className={`flex gap-3 max-w-3xl ${
                  m.role === 'user' ? 'ml-auto justify-end' : 'mr-auto justify-start'
                }`}
              >
                {m.role !== 'user' && (
                  <div className="w-8 h-8 rounded-full bg-blue-500/15 text-blue-600 dark:text-blue-400 flex items-center justify-center shrink-0 mt-0.5">
                    <Bot className="w-4 h-4" />
                  </div>
                )}

                <div
                  className={`p-3.5 rounded-xl text-sm leading-relaxed space-y-2 max-w-2xl ${
                    m.role === 'user'
                      ? 'bg-blue-600 text-white rounded-tr-none shadow-sm'
                      : m.role === 'tool'
                      ? 'bg-neutral-100 dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-neutral-800 dark:text-neutral-200 font-mono text-xs'
                      : 'bg-white dark:bg-neutral-850 border border-neutral-200 dark:border-neutral-750 text-neutral-900 dark:text-neutral-100 rounded-tl-none shadow-xs'
                  }`}
                >
                  {m.role === 'tool' ? (
                    <div className="flex items-center gap-1.5 text-xs text-blue-600 dark:text-blue-400 font-sans font-medium">
                      <Wrench className="w-3.5 h-3.5" />
                      Tool Result: <code>{m.tool_name || 'internal_tool'}</code>
                    </div>
                  ) : null}

                  <div className="whitespace-pre-wrap break-words">{m.content}</div>

                  <div
                    className={`text-[10px] flex items-center gap-1 pt-1 ${
                      m.role === 'user' ? 'text-white/80 justify-end' : 'text-neutral-500 dark:text-neutral-400'
                    }`}
                  >
                    <Clock className="w-2.5 h-2.5" />
                    {new Date(m.created_at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
                  </div>
                </div>

                {m.role === 'user' && (
                  <div className="w-8 h-8 rounded-full bg-blue-600/20 text-blue-600 dark:text-blue-400 flex items-center justify-center shrink-0 mt-0.5 font-bold text-xs">
                    <User className="w-4 h-4" />
                  </div>
                )}
              </div>
            ))
        )}

        {chatMutation.isPending && (
          <div className="flex gap-3 max-w-3xl mr-auto justify-start animate-in fade-in-0">
            <div className="w-8 h-8 rounded-full bg-blue-500/15 text-blue-600 dark:text-blue-400 flex items-center justify-center shrink-0 mt-0.5">
              <Bot className="w-4 h-4" />
            </div>
            <div className="p-4 rounded-xl text-sm bg-white dark:bg-neutral-850 border border-neutral-200 dark:border-neutral-800 text-neutral-900 dark:text-neutral-100 rounded-tl-none shadow-2xs flex items-center gap-2">
              <RefreshCw className="w-4 h-4 animate-spin text-blue-600 dark:text-blue-400" />
              <span className="text-xs text-neutral-500 dark:text-neutral-400">Thinking and querying tools...</span>
            </div>
          </div>
        )}

        {chatMutation.error && (
          <div className="p-3.5 bg-red-50 dark:bg-red-950/40 border border-red-200 dark:border-red-800/50 rounded-lg text-red-800 dark:text-red-300 text-xs flex items-start gap-2 max-w-2xl mx-auto">
            <AlertCircle className="w-4 h-4 shrink-0 mt-0.5 text-red-500" />
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
          className="w-full px-4 py-3 pr-14 bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 rounded-xl text-sm focus:outline-none focus:ring-2 focus:ring-blue-500/40 resize-none shadow-xs"
        />
        <button
          type="submit"
          disabled={!prompt.trim() || chatMutation.isPending}
          className="absolute right-3 bottom-3 p-2 bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 rounded-lg hover:bg-neutral-800 dark:hover:bg-neutral-200 transition-colors disabled:opacity-40 cursor-pointer"
        >
          <Send className="w-4 h-4" />
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
