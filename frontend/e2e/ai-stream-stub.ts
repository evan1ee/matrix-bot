/**
 * A `window.fetch` stub for `POST /api/v1/ai/chat/stream`.
 *
 * `page.route` + `route.fulfill` sends a whole body at once, so it cannot drip
 * text, split a character across chunks or hold a stream open until Stop. The
 * stub is installed with `addInitScript`; every other request goes to the real
 * `fetch`, so the `page.route` fakes keep working.
 *
 * Without a script the stub falls back to the plain `POST /ai/chat` fake and
 * replays its answer as `started`, `delta`, `done`, so specs written against
 * the non-streaming endpoint keep their persisted messages and tool rows.
 */

/** One step of a scripted stream: an event (or raw text) and when to send it. */
export type StreamStep = {
  /** An `AiStreamEvent`, sent as one `data:` frame. */
  event?: Record<string, unknown>;
  /** Raw text sent as is, for keep-alive comments. */
  raw?: string;
  /** Wait this long before the step. */
  delayMs?: number;
  /** Split the encoded frame after this many bytes, into two chunks. */
  splitAtByte?: number;
};

export type StreamScript = {
  /** A non-2xx answer: sent as JSON instead of a stream. */
  /** Fail at the network level, as a refused connection does. */
  networkError?: boolean;
  status?: number;
  errorBody?: unknown;
  steps?: StreamStep[];
  /**
   * Keep the stream open after the steps, until the test calls `send`/`end` or
   * the page aborts the request.
   */
  hold?: boolean;
};

type Controller = ReadableStreamDefaultController<Uint8Array>;

type StubState = {
  scripts: StreamScript[];
  bodies: unknown[];
  aborts: number;
  open: Controller | null;
  send: (frame: Record<string, unknown>) => void;
  end: () => void;
};

declare global {
  interface Window {
    __aiStream: StubState;
  }
}

/** Runs in the page: must not reference anything outside this function. */
export function installAiStreamStub() {
  const encoder = new TextEncoder();
  const realFetch = window.fetch.bind(window);
  const state: StubState = {
    scripts: [],
    bodies: [],
    aborts: 0,
    open: null,
    send: (frame) => state.open?.enqueue(encoder.encode(`data: ${JSON.stringify(frame)}\n\n`)),
    end: () => {
      state.open?.close();
      state.open = null;
    },
  };
  window.__aiStream = state;

  const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
  const sseResponse = (stream: ReadableStream<Uint8Array>) =>
    new Response(stream, { status: 200, headers: { 'Content-Type': 'text/event-stream' } });

  async function replayPlainChat(input: RequestInfo | URL, init: RequestInit): Promise<Response> {
    const plainUrl = String(input).replace('/ai/chat/stream', '/ai/chat');
    const res = await realFetch(plainUrl, init);
    if (!res.ok) return res;
    const data = await res.json();
    const frames = [
      { type: 'started', conversation_id: data.conversation_id },
      { type: 'delta', text: data.message },
      { type: 'segment_end' },
      {
        type: 'done',
        conversation_id: data.conversation_id,
        message: data.message,
        model_used: data.model_used,
        tool_calls_executed: 0,
      },
    ];
    const body = frames.map((f) => `data: ${JSON.stringify(f)}\n\n`).join('');
    return sseResponse(
      new ReadableStream({
        start(controller) {
          controller.enqueue(encoder.encode(body));
          controller.close();
        },
      })
    );
  }

  window.fetch = async (input: RequestInfo | URL, init: RequestInit = {}) => {
    const url = String(input);
    if (!url.includes('/api/v1/ai/chat/stream') || (init.method ?? 'GET') !== 'POST') {
      return realFetch(input, init);
    }
    const script = state.scripts.shift();
    if (!script) return replayPlainChat(input, init);

    state.bodies.push(JSON.parse(String(init.body ?? 'null')));
    if (script.networkError) throw new TypeError('Failed to fetch');
    if (script.status && script.status >= 400) {
      return new Response(JSON.stringify(script.errorBody ?? { error: 'failed', code: script.status }), {
        status: script.status,
        headers: { 'Content-Type': 'application/json' },
      });
    }

    const signal = init.signal ?? undefined;
    return sseResponse(
      new ReadableStream<Uint8Array>({
        async start(controller) {
          let aborted = false;
          const onAbort = () => {
            aborted = true;
            state.aborts += 1;
            state.open = null;
            controller.error(new DOMException('The operation was aborted.', 'AbortError'));
          };
          if (signal?.aborted) return onAbort();
          signal?.addEventListener('abort', onAbort, { once: true });

          for (const step of script.steps ?? []) {
            if (step.delayMs) await sleep(step.delayMs);
            if (aborted) return;
            const text = step.raw ?? `data: ${JSON.stringify(step.event)}\n\n`;
            const bytes = encoder.encode(text);
            if (step.splitAtByte !== undefined) {
              controller.enqueue(bytes.slice(0, step.splitAtByte));
              await sleep(20);
              if (aborted) return;
              controller.enqueue(bytes.slice(step.splitAtByte));
            } else {
              controller.enqueue(bytes);
            }
          }
          if (script.hold) {
            state.open = controller;
          } else if (!aborted) {
            controller.close();
          }
        },
      })
    );
  };
}
