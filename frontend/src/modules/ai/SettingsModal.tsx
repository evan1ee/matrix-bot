import React, { useState, useEffect } from 'react';
import { Settings, AlertCircle } from 'lucide-react';
import { AiUserConfig } from './types';

interface AiSettingsModalProps {
  isOpen: boolean;
  onClose: () => void;
  config?: AiUserConfig | null;
  onSave: (config: Partial<AiUserConfig>) => void;
  isSaving: boolean;
  error?: string | null;
}

export const AiSettingsModal: React.FC<AiSettingsModalProps> = ({
  isOpen,
  onClose,
  config,
  onSave,
  isSaving,
  error,
}) => {
  const [baseUrl, setBaseUrl] = useState('');
  const [apiKey, setApiKey] = useState('');
  const [model, setModel] = useState('');
  const [systemPrompt, setSystemPrompt] = useState('');
  const [temperature, setTemperature] = useState<number>(0.7);

  useEffect(() => {
    if (config) {
      setBaseUrl(config.api_base_url || '');
      setApiKey('');
      setModel(config.model_name || '');
      setSystemPrompt(config.system_prompt || '');
      setTemperature(config.temperature ?? 0.7);
    }
  }, [config, isOpen]);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && isOpen) {
        onClose();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    onSave({
      api_base_url: baseUrl.trim() || undefined,
      api_key: apiKey.trim() || undefined,
      model_name: model.trim() || undefined,
      system_prompt: systemPrompt.trim() || undefined,
      temperature,
    });
  };

  return (
    <div
      className="fixed inset-0 z-50 bg-black/60 backdrop-blur-xs flex items-center justify-center p-4"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="bg-card text-card-foreground w-full max-w-lg rounded-xl border border-border shadow-2xl overflow-hidden animate-in fade-in-0 zoom-in-95">
        <div className="p-5 border-b border-border flex items-center justify-between">
          <h2 className="text-base font-bold text-foreground flex items-center gap-2">
            <Settings className="w-4 h-4 text-primary" />
            AI Assistant Settings
          </h2>
          <button
            onClick={onClose}
            className="text-xs text-muted-foreground hover:text-foreground cursor-pointer"
          >
            Close
          </button>
        </div>

        {error && (
          <div className="mx-5 mt-4 p-3 rounded-lg bg-destructive/10 border border-destructive/20 text-destructive text-xs flex items-center gap-2">
            <AlertCircle className="w-4 h-4 shrink-0" />
            <span>{error}</span>
          </div>
        )}

        <form onSubmit={handleSubmit} className="p-5 space-y-4">
          <div>
            <label className="block text-xs font-medium text-foreground mb-1">
              API Endpoint Base URL
            </label>
            <input
              type="text"
              placeholder="http://localhost:11434/v1 (Ollama) or https://api.openai.com/v1"
              value={baseUrl}
              onChange={(e) => setBaseUrl(e.target.value)}
              className="w-full px-3 py-2 bg-background border border-input rounded-lg text-xs font-mono select-all"
            />
            <p className="text-[11px] text-muted-foreground mt-1">
              Compatible with Ollama, LocalAI, vLLM, OpenAI, Groq, DeepSeek, and OpenRouter.
            </p>
          </div>

          <div>
            <label className="block text-xs font-medium text-foreground mb-1">
              API Key {config?.has_api_key && <span className="text-emerald-500 font-normal ml-1">✓ Saved</span>}
            </label>
            <input
              type="password"
              placeholder={config?.has_api_key ? '•••••••• (Enter new key to replace)' : 'sk-...'}
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              className="w-full px-3 py-2 bg-background border border-input rounded-lg text-xs font-mono"
            />
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="block text-xs font-medium text-foreground mb-1">
                Model Name
              </label>
              <input
                type="text"
                placeholder="llama3.2, gpt-4o-mini, deepseek-chat"
                value={model}
                onChange={(e) => setModel(e.target.value)}
                className="w-full px-3 py-2 bg-background border border-input rounded-lg text-xs font-mono"
              />
            </div>
            <div>
              <label className="block text-xs font-medium text-foreground mb-1">
                Temperature ({temperature})
              </label>
              <input
                type="range"
                min="0"
                max="2"
                step="0.1"
                value={temperature}
                onChange={(e) => setTemperature(parseFloat(e.target.value))}
                className="w-full mt-2"
              />
            </div>
          </div>

          <div>
            <label className="block text-xs font-medium text-foreground mb-1">
              Custom System Prompt (Optional)
            </label>
            <textarea
              rows={3}
              placeholder="You are an assistant who answers concisely..."
              value={systemPrompt}
              onChange={(e) => setSystemPrompt(e.target.value)}
              className="w-full px-3 py-2 bg-background border border-input rounded-lg text-xs"
            />
          </div>

          <div className="pt-3 border-t border-border flex justify-end gap-2">
            <button
              type="button"
              onClick={onClose}
              className="px-4 py-2 text-xs font-medium text-muted-foreground hover:text-foreground bg-muted rounded-lg transition-colors cursor-pointer"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={isSaving}
              className="px-4 py-2 text-xs font-medium bg-primary text-primary-foreground hover:bg-primary/90 rounded-lg transition-colors disabled:opacity-50 cursor-pointer"
            >
              {isSaving ? 'Saving...' : 'Save Settings'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
