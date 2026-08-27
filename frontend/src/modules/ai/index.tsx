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
      <div className="flex items-center justify-between gap-3 border-b border-border pb-4">
        <div className="flex items-center gap-2.5">
          <div className="w-9 h-9 rounded-lg bg-primary/10 text-primary flex items-center justify-center">
            <Sparkles className="w-5 h-5" />
          </div>
          <div>
            <h1 className="text-xl font-bold tracking-tight text-foreground flex items-center gap-2">
              AI Assistant
              <span className="font-mono text-[11px] font-normal px-2 py-0.5 rounded-full bg-muted text-muted-foreground border border-border">
                {config?.model_name || 'llama3.2'}
              </span>
            </h1>
            <p className="text-xs text-muted-foreground">
              Conversational assistant with read-only access to your tasks, reminders, bills, and finances.
            </p>
          </div>
        </div>

        <div className="flex items-center gap-2">
          {activeConversationId && (
            <button
              onClick={() => resetMutation.mutate(activeConversationId)}
              disabled={resetMutation.isPending}
              title="Reset current conversation"
              className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium bg-muted hover:bg-muted/80 text-foreground rounded-lg transition-colors cursor-pointer"
            >
              <Trash2 className="w-3.5 h-3.5 text-muted-foreground" />
              New Chat
            </button>
          )}

          <button
            onClick={() => setIsSettingsOpen(true)}
            title="Configure AI model & API keys"
            className="p-2 text-muted-foreground hover:text-foreground bg-muted/60 hover:bg-muted rounded-lg transition-colors cursor-pointer"
          >
            <Settings className="w-4 h-4" />
          </button>
        </div>
      </div>

      {/* Messages Thread Container */}
      <div className="flex-1 overflow-y-auto rounded-xl border border-border bg-card/40 p-4 sm:p-6 space-y-5">
        {isLoadingMessages ? (
          <div className="py-20 text-center text-sm text-muted-foreground">Loading conversation...</div>
        ) : messages.length === 0 ? (
          <div className="py-16 text-center max-w-lg mx-auto space-y-6">
            <div className="w-12 h-12 rounded-full bg-primary/10 text-primary flex items-center justify-center mx-auto">
              <Bot className="w-6 h-6" />
            </div>
            <div>
              <h3 className="text-base font-semibold text-foreground">How can I help you today?</h3>
              <p className="text-xs text-muted-foreground mt-1">
                Ask questions, explore ideas, or ask me to check your tasks, reminders, bills, and financial ledger.
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
                  className="p-3 rounded-lg border border-border bg-background hover:bg-muted/60 text-xs text-foreground transition-colors flex items-center justify-between group cursor-pointer"
                >
                  <span className="line-clamp-2">{s}</span>
                  <ChevronRight className="w-3.5 h-3.5 text-muted-foreground group-hover:text-primary shrink-0 ml-1.5" />
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
                  <div className="w-8 h-8 rounded-full bg-primary/15 text-primary flex items-center justify-center shrink-0 mt-0.5">
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
            <div className="w-8 h-8 rounded-full bg-primary/15 text-primary flex items-center justify-center shrink-0 mt-0.5">
              <Bot className="w-4 h-4" />
            </div>
            <div className="p-4 rounded-xl text-sm bg-card border border-border text-card-foreground rounded-tl-none shadow-2xs flex items-center gap-2">
              <RefreshCw className="w-4 h-4 animate-spin text-primary" />
              <span className="text-xs text-muted-foreground">Thinking and querying tools...</span>
            </div>
          </div>
        )}

        {chatMutation.error && (
          <div className="p-3.5 bg-destructive/10 border border-destructive/20 rounded-lg text-destructive text-xs flex items-start gap-2 max-w-2xl mx-auto">
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
          className="w-full px-4 py-3 pr-14 bg-card border border-input rounded-xl text-sm focus:outline-hidden focus:ring-2 focus:ring-primary/40 resize-none"
        />
        <button
          type="submit"
          disabled={!prompt.trim() || chatMutation.isPending}
          className="absolute right-3 bottom-3 p-2 bg-primary text-primary-foreground rounded-lg hover:bg-primary/90 transition-colors disabled:opacity-40 cursor-pointer"
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
