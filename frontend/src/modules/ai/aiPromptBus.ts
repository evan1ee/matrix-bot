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

export function registerAiPromptListener(listener: (prompt: string, execute: boolean) => void): () => void {
  promptListener = listener;
  if (pendingAiPrompt) {
    const { prompt, execute } = pendingAiPrompt;
    pendingAiPrompt = null;
    listener(prompt, execute);
  }
  return () => {
    promptListener = null;
  };
}
