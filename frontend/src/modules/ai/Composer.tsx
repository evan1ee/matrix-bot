import React from 'react';
import { Send, Square } from 'lucide-react';
import { VoiceDictateButton } from '../../components/VoiceDictateButton';
import type { LiveTurn } from './liveTurn';

/**
 * The input row. Send turns into Stop while a turn runs; the textarea stays
 * editable so the next prompt can be drafted, but cannot be sent until the
 * turn ends. Escape in the textarea also stops it.
 */
export const Composer: React.FC<{
  prompt: string;
  onPromptChange: (value: string | ((prev: string) => string)) => void;
  onSend: () => void;
  onStop: () => void;
  phase: LiveTurn['phase'] | null;
  inputRef: React.RefObject<HTMLTextAreaElement | null>;
}> = ({ prompt, onPromptChange, onSend, onStop, phase, inputRef }) => {
  const running = phase === 'running';
  const active = running || phase === 'stopping';

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Escape' && running) {
      e.preventDefault();
      onStop();
    } else if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      onSend();
    }
  };

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        onSend();
      }}
      className="relative"
    >
      <textarea
        ref={inputRef}
        rows={2}
        value={prompt}
        onChange={(e) => onPromptChange(e.target.value)}
        onKeyDown={handleKeyDown}
        placeholder="Ask AI assistant... (Enter to send, Shift+Enter for newline)"
        className="w-full px-3.5 py-2.5 pr-28 bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 rounded-lg text-sm focus:outline-none focus:border-neutral-500 font-sans resize-none shadow-2xs"
      />
      <div className="absolute right-2.5 bottom-2.5 flex items-center gap-1.5">
        <VoiceDictateButton
          onTranscript={(text) => onPromptChange((prev) => (prev.trim() ? `${prev.trim()} ${text}` : text))}
          disabled={active}
        />
        {running ? (
          <button
            type="button"
            onClick={() => {
              onStop();
              // The button goes away with the running turn; keep focus in the input.
              inputRef.current?.focus();
            }}
            aria-label="Stop"
            title="Stop (Esc)"
            className="inline-flex items-center gap-1.5 px-2 py-1.5 text-xs font-medium border border-neutral-300 dark:border-neutral-700 text-neutral-800 dark:text-neutral-200 rounded hover:bg-neutral-100 dark:hover:bg-neutral-800 transition-colors duration-150 cursor-pointer focus-visible:outline focus-visible:outline-2 focus-visible:outline-neutral-500"
          >
            <Square className="w-3 h-3 fill-current" aria-hidden />
            <span>Stop</span>
          </button>
        ) : (
          <button
            type="submit"
            disabled={!prompt.trim() || active}
            className="p-1.5 bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 rounded hover:bg-neutral-800 dark:hover:bg-neutral-200 transition-colors duration-150 disabled:opacity-40 cursor-pointer"
            title="Send message"
          >
            <Send className="w-3.5 h-3.5" />
          </button>
        )}
      </div>
    </form>
  );
};
