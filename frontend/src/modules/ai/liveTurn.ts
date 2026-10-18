import type { QueryClient } from '@tanstack/react-query';
import { ApiHttpError, fetchApi, streamApi } from '../../core/apiClient';
import type { components } from '../../types/api';
import type { AiMessage } from './types';

/**
 * The AI turn that is in flight, kept outside React so that leaving the AI
 * page does not stop it. Only `stopTurn` (or closing the tab) ends a turn
 * early; the view merely subscribes.
 */

export type AiStreamEvent = components['schemas']['AiStreamEvent'];
export type AiToolOutcome = components['schemas']['AiToolOutcome'];

export type LivePart =
  | { kind: 'text'; text: string; closed: boolean }
  | { kind: 'tool'; callId: string; name: string; outcome: 'running' | AiToolOutcome };

export type LiveTurn = {
  prompt: string;
  conversationId: string | null;
  parts: LivePart[];
  phase: 'running' | 'stopping' | 'done' | 'error';
  error?: string;
};

/**
 * How long to wait for the server's `AiConversationUpdated` event after the
 * stream ended without a terminal event (Stop, a server restart, a dropped
 * connection). It matches the server's own turn limit.
 */
const TAIL_TIMEOUT_MS = 180_000;

let turn: LiveTurn | null = null;
/** Bumped for every turn, so a late callback from an earlier one is ignored. */
let runId = 0;
let controller: AbortController | null = null;
let tailTimer: ReturnType<typeof setTimeout> | null = null;
let queryClient: QueryClient | null = null;
const listeners = new Set<() => void>();

function setTurn(next: LiveTurn | null) {
  turn = next;
  listeners.forEach((listener) => listener());
}

function updateTurn(change: (current: LiveTurn) => LiveTurn) {
  if (turn) setTurn(change(turn));
}

export function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getSnapshot(): LiveTurn | null {
  return turn;
}

/** The store refetches history through the app's query client; the app hands it over here. */
export function bindQueryClient(client: QueryClient) {
  queryClient = client;
}

function cancelTail() {
  if (tailTimer !== null) {
    clearTimeout(tailTimer);
    tailTimer = null;
  }
}

/** Pure reducer for one stream event; `done` and `error` are handled by the caller. */
function applyEvent(current: LiveTurn, event: AiStreamEvent): LiveTurn {
  const parts = current.parts;
  const last = parts[parts.length - 1];
  switch (event.type) {
    case 'started':
      return { ...current, conversationId: event.conversation_id };
    case 'delta':
      if (last?.kind === 'text' && !last.closed) {
        return { ...current, parts: [...parts.slice(0, -1), { ...last, text: last.text + event.text }] };
      }
      return { ...current, parts: [...parts, { kind: 'text', text: event.text, closed: false }] };
    case 'segment_end':
      if (last?.kind === 'text' && !last.closed) {
        return { ...current, parts: [...parts.slice(0, -1), { ...last, closed: true }] };
      }
      return current;
    case 'segment_retracted': {
      const index = parts.map((p) => p.kind === 'text' && p.closed).lastIndexOf(true);
      return index < 0 ? current : { ...current, parts: parts.filter((_, i) => i !== index) };
    }
    case 'tool_started':
      return {
        ...current,
        parts: [...parts, { kind: 'tool', callId: event.call_id, name: event.name, outcome: 'running' }],
      };
    case 'tool_finished': {
      const known = parts.some((p) => p.kind === 'tool' && p.callId === event.call_id);
      const finished: LivePart = { kind: 'tool', callId: event.call_id, name: event.name, outcome: event.outcome };
      return {
        ...current,
        parts: known ? parts.map((p) => (p.kind === 'tool' && p.callId === event.call_id ? finished : p)) : [...parts, finished],
      };
    }
    default:
      return current;
  }
}

/** Pulls the stored history in, then drops the live turn so the stored messages take over. */
async function finish(id: number, conversationId: string | null) {
  cancelTail();
  if (queryClient && conversationId) {
    const client = queryClient;
    try {
      await Promise.all([
        client.invalidateQueries({ queryKey: ['ai-conversations'] }),
        client.fetchQuery({
          queryKey: ['ai-messages', conversationId],
          queryFn: () => fetchApi<AiMessage[]>(`/ai/conversations/${conversationId}/messages`),
          staleTime: 0,
        }),
      ]);
    } catch {
      // The history refetches on its own the next time the view asks for it.
    }
  }
  // Let the query cache notify its observers before the live bubble goes, so
  // the reply never vanishes for a frame between the two.
  await new Promise((resolve) => setTimeout(resolve, 0));
  if (id === runId) {
    controller = null;
    setTurn(null);
  }
}

/** The stream is over without `done`/`error`: wait for the server's update event, or give up. */
function beginTail(id: number) {
  if (id !== runId || !turn) return;
  cancelTail();
  updateTurn((t) => ({ ...t, phase: 'stopping' }));
  tailTimer = setTimeout(() => void finish(id, turn?.conversationId ?? null), TAIL_TIMEOUT_MS);
}

function handleEvent(id: number, event: AiStreamEvent) {
  if (id !== runId || turn?.phase !== 'running') return;
  if (event.type === 'done') {
    const hasText = turn.parts.some((p) => p.kind === 'text');
    updateTurn((t) => ({
      ...t,
      conversationId: event.conversation_id,
      phase: 'done',
      parts:
        hasText || !event.message
          ? t.parts
          : [...t.parts, { kind: 'text', text: event.message, closed: true }],
    }));
    void finish(id, event.conversation_id);
  } else if (event.type === 'error') {
    updateTurn((t) => ({ ...t, phase: 'error', error: event.message }));
  } else {
    updateTurn((t) => applyEvent(t, event));
  }
}

async function run(id: number, body: unknown, signal: AbortSignal) {
  // `turn.conversationId` is seeded by the caller, so it cannot tell whether
  // the server ever accepted the turn; only a received `started` can.
  let started = false;
  const fail = (message: string) => updateTurn((t) => ({ ...t, phase: 'error', error: message }));
  try {
    await streamApi<AiStreamEvent>('/ai/chat/stream', body, {
      signal,
      onEvent: (event) => {
        if (event.type === 'started') started = true;
        handleEvent(id, event);
      },
    });
  } catch (err) {
    if (id !== runId || turn?.phase !== 'running') return; // Stop already took over
    if (err instanceof DOMException && err.name === 'AbortError') return;
    // Before `started` nothing runs on the server, so the request simply
    // failed (rejected start, refused connection). After it, the turn may
    // still be running there and only its stored result will tell.
    if (started && !(err instanceof ApiHttpError)) {
      beginTail(id);
      return;
    }
    fail(err instanceof Error ? err.message : 'The request failed');
    return;
  }
  if (id !== runId || turn?.phase !== 'running') return;
  if (started) beginTail(id);
  else fail('The server closed the connection before replying');
}

/**
 * Starts a turn. Returns false, doing nothing, while another one is running
 * or being stopped: a tab has one live turn at a time.
 */
export function startTurn(prompt: string, conversationId?: string | null, roomId?: string | null): boolean {
  if (turn && (turn.phase === 'running' || turn.phase === 'stopping')) return false;
  cancelTail();
  runId += 1;
  controller = new AbortController();
  setTurn({ prompt, conversationId: conversationId ?? null, parts: [], phase: 'running' });
  void run(
    runId,
    { prompt, conversation_id: conversationId ?? undefined, room_id: roomId ?? undefined },
    controller.signal
  );
  return true;
}

/** Aborts the stream. The server finishes its safe point and stores the turn; we wait for that. */
export function stopTurn() {
  if (!turn || turn.phase !== 'running') return;
  controller?.abort();
  beginTail(runId);
}

/** The server stored a conversation update; ends a stopping turn that waits for it. */
export function notifyConversationUpdated(conversationId: string) {
  if (turn?.phase !== 'stopping') return;
  if (turn.conversationId !== null && turn.conversationId !== conversationId) return;
  void finish(runId, conversationId);
}

/** Forgets the turn (an error notice the user has seen); a running stream is aborted. */
export function clearTurn() {
  cancelTail();
  controller?.abort();
  controller = null;
  runId += 1;
  setTurn(null);
}
