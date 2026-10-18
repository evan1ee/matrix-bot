import React, { useLayoutEffect, useEffect, useRef } from 'react';
import { Bot, ChevronRight, Clock } from 'lucide-react';
import { MarkdownRenderer } from '../../components/MarkdownRenderer';
import { formatShortTimeInZone } from '../../core/settings';
import type { AiMessage } from './types';
import type { LiveTurn } from './liveTurn';
import { AssistantRow, UserBubble } from './Bubbles';
import { LiveTurnBubble } from './LiveTurnBubble';
import { ToolStep } from './ToolStep';

const SUGGESTIONS = [
  'Spent $16.50 on lunch at Shake Shack, check off my workout habit, and remind me to call mom at 7pm',
  'Log my mood as amazing today, spent $45 on groceries, and add task Buy birthday gift',
  'Generate weekly retrospective',
  'What tasks do I have due today?',
  'Show a breakdown of my portfolio and cash balance',
  'Summarize my upcoming reminders and bills',
];

function StoredMessage({ m, timezone }: { m: AiMessage; timezone: string }) {
  const time = formatShortTimeInZone(m.created_at, timezone);
  if (m.role === 'user') return <UserBubble content={m.content} time={time} />;
  if (m.role === 'tool') {
    return (
      <div className="max-w-3xl mr-auto pl-10">
        <ToolStep name={m.tool_name || 'internal_tool'} outcome={m.tool_outcome ?? null} result={m.content} />
      </div>
    );
  }
  return (
    <AssistantRow>
      <div className="p-3.5 rounded-lg text-sm leading-relaxed space-y-2 max-w-2xl font-sans bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-neutral-900 dark:text-neutral-100 shadow-xs">
        <MarkdownRenderer content={m.content} />
        <div className="text-[10px] font-mono flex items-center gap-1 pt-1 text-neutral-400 dark:text-neutral-500">
          <Clock className="w-2.5 h-2.5" />
          <span>{time}</span>
        </div>
      </div>
    </AssistantRow>
  );
}

/** The scrolling thread: stored history, then the live turn; owns the scroll behaviour. */
export const MessageList: React.FC<{
  messages: AiMessage[];
  isLoading: boolean;
  conversationId: string | null;
  turn: LiveTurn | null;
  timezone: string;
  onSuggestion: (text: string) => void;
}> = ({ messages, isLoading, conversationId, turn, timezone, onSuggestion }) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const prevLengthRef = useRef(0);
  const prevConvIdRef = useRef<string | null>(null);

  // Jump to the bottom when messages load or the conversation changes, so the
  // smooth scroll never animates through the whole history.
  useLayoutEffect(() => {
    const el = containerRef.current;
    if (!el || messages.length === 0) return;

    let raf: number | null = null;
    const isDifferentConv = prevConvIdRef.current !== conversationId;
    const isInitialLoad = prevLengthRef.current === 0 && messages.length > 0;

    if (isDifferentConv || isInitialLoad) {
      el.scrollTop = el.scrollHeight;
      raf = requestAnimationFrame(() => {
        if (containerRef.current) containerRef.current.scrollTop = containerRef.current.scrollHeight;
      });
    } else if (messages.length > prevLengthRef.current) {
      el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' });
    }

    prevLengthRef.current = messages.length;
    prevConvIdRef.current = conversationId;
    return () => {
      if (raf !== null) cancelAnimationFrame(raf);
    };
  }, [messages, conversationId]);

  useEffect(() => {
    if (turn && containerRef.current) {
      containerRef.current.scrollTo({ top: containerRef.current.scrollHeight, behavior: 'smooth' });
    }
  }, [turn?.parts, turn?.phase]);

  // When a finished turn's stored messages arrive, they replace its live
  // bubble at once instead of both showing for the frames until the store
  // drops the turn. The stored count while no turn runs is the baseline.
  const idleCountRef = useRef<number | null>(null);
  if (!turn) idleCountRef.current = messages.length;
  const handedOver =
    turn?.phase === 'done' && idleCountRef.current !== null && messages.length > idleCountRef.current;

  const visible = messages.filter(
    (m) => m.role !== 'system' && (m.role !== 'assistant' || m.content.trim().length > 0)
  );

  return (
    <div
      ref={containerRef}
      className="flex-1 overflow-y-auto rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/40 dark:bg-neutral-900/30 p-4 sm:p-6 space-y-5"
    >
      {isLoading && messages.length === 0 && !turn ? (
        <div className="py-20 text-center text-xs font-mono text-neutral-500 dark:text-neutral-400">Loading conversation...</div>
      ) : messages.length === 0 && !turn ? (
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
            {SUGGESTIONS.map((s) => (
              <button
                key={s}
                type="button"
                onClick={() => onSuggestion(s)}
                className="p-3 rounded-md border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 hover:border-neutral-400 dark:hover:border-neutral-700 text-xs text-neutral-800 dark:text-neutral-200 transition-colors duration-150 flex items-center justify-between group cursor-pointer shadow-2xs font-sans"
              >
                <span className="line-clamp-2">{s}</span>
                <ChevronRight className="w-3.5 h-3.5 text-neutral-400 group-hover:text-neutral-900 dark:group-hover:text-neutral-100 shrink-0 ml-1.5 transition-colors duration-150" />
              </button>
            ))}
          </div>
        </div>
      ) : (
        visible.map((m) => <StoredMessage key={m.id} m={m} timezone={timezone} />)
      )}

      {turn && !handedOver && <LiveTurnBubble turn={turn} />}
    </div>
  );
};
