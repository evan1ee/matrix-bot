import React from 'react';
import { AlertCircle, RefreshCw } from 'lucide-react';
import { MarkdownRenderer } from '../../components/MarkdownRenderer';
import type { LiveTurn } from './liveTurn';
import { AssistantRow, UserBubble } from './Bubbles';
import { ToolStep } from './ToolStep';

/** The turn in flight: the prompt, the reply as it arrives (text and tool steps), and its status. */
export const LiveTurnBubble: React.FC<{ turn: LiveTurn }> = ({ turn }) => {
  const stepRunning = turn.parts.some((p) => p.kind === 'tool' && p.outcome === 'running');
  const status =
    turn.phase === 'stopping'
      ? stepRunning
        ? 'Stopping… finishing the current step'
        : 'Stopping…'
      : turn.phase === 'running' && turn.parts.length === 0
      ? 'Thinking…'
      : null;

  return (
    <>
      <UserBubble content={turn.prompt} />

      {turn.parts.length > 0 && (
        <AssistantRow>
          <div
            data-testid="live-turn"
            className="p-3.5 rounded-lg text-sm leading-relaxed space-y-2 max-w-2xl font-sans bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 text-neutral-900 dark:text-neutral-100 shadow-xs"
          >
            {turn.parts.map((part, i) =>
              part.kind === 'text' ? (
                <MarkdownRenderer key={i} content={part.text} />
              ) : (
                <ToolStep key={part.callId} name={part.name} outcome={part.outcome} />
              )
            )}
          </div>
        </AssistantRow>
      )}

      {status && (
        <AssistantRow>
          <div
            role="status"
            className="p-3.5 rounded-lg text-sm bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 shadow-2xs flex items-center gap-2"
          >
            <RefreshCw className="w-3.5 h-3.5 animate-spin text-neutral-500" aria-hidden />
            <span className="text-xs text-neutral-500 font-sans">{status}</span>
          </div>
        </AssistantRow>
      )}

      {turn.phase === 'error' && (
        <div className="p-3 bg-red-500/10 border border-red-500/20 rounded-md text-red-600 dark:text-red-400 text-xs flex items-start gap-2 max-w-2xl mx-auto font-sans">
          <AlertCircle className="w-4 h-4 shrink-0 mt-0.5" />
          <div>
            <strong>Generation failed:</strong> {turn.error}
          </div>
        </div>
      )}
    </>
  );
};
