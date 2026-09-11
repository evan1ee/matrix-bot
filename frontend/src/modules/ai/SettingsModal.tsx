import React, { useState, useEffect } from 'react';
import { Settings, AlertCircle, X, Check, ShieldCheck, RefreshCw } from 'lucide-react';
import { AiUserConfig, VerifyAiKeyResponse } from './types';
import { CustomSelect } from '../../components/CustomSelect';
import { AI_PROVIDERS, findProviderByUrl } from './providers';
import { fetchApi } from '../../core/apiClient';

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
  const [providerId, setProviderId] = useState('openai');
  const [customBaseUrl, setCustomBaseUrl] = useState('');
  const [apiKey, setApiKey] = useState('');
  const [selectedModel, setSelectedModel] = useState('gpt-4o-mini');
  const [customModelText, setCustomModelText] = useState('');
  const [systemPrompt, setSystemPrompt] = useState('');
  const [temperature, setTemperature] = useState<number>(0.7);
  const [maxHistoryTurns, setMaxHistoryTurns] = useState<number>(10);
  const [isVerifying, setIsVerifying] = useState(false);
  const [verifyResult, setVerifyResult] = useState<VerifyAiKeyResponse | null>(null);

  useEffect(() => {
    if (config) {
      const initialProvider = findProviderByUrl(config.api_base_url);
      setProviderId(initialProvider.id);
      setCustomBaseUrl(initialProvider.id === 'custom' ? config.api_base_url || '' : '');

      const currentModel = config.model_name || initialProvider.defaultModel || 'gpt-4o-mini';
      const existsInProvider = initialProvider.models.some((m) => m.value === currentModel);
      if (existsInProvider) {
        setSelectedModel(currentModel);
        setCustomModelText('');
      } else {
        setSelectedModel('__custom__');
        setCustomModelText(currentModel);
      }

      setApiKey('');
      setSystemPrompt(config.system_prompt || '');
      setTemperature(config.temperature ?? 0.7);
      setMaxHistoryTurns(config.max_history_turns ?? 10);
    } else {
      setProviderId('openai');
      setSelectedModel('gpt-4o-mini');
      setCustomBaseUrl('');
      setCustomModelText('');
      setApiKey('');
      setSystemPrompt('');
      setTemperature(0.7);
      setMaxHistoryTurns(10);
    }
    setVerifyResult(null);
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

  const activeProvider = AI_PROVIDERS.find((p) => p.id === providerId) || AI_PROVIDERS[0];

  const handleProviderChange = (newId: string) => {
    setProviderId(newId);
    setVerifyResult(null);
    const newProvider = AI_PROVIDERS.find((p) => p.id === newId) || AI_PROVIDERS[0];
    if (newId === 'custom') {
      setSelectedModel('__custom__');
    } else {
      const exists = newProvider.models.some((m) => m.value === selectedModel);
      if (!exists) {
        setSelectedModel(newProvider.defaultModel);
        setCustomModelText('');
      }
    }
  };

  const handleModelChange = (val: string) => {
    setSelectedModel(val);
    setVerifyResult(null);
    if (val !== '__custom__') {
      setCustomModelText('');
    }
  };

  const handleVerify = async () => {
    setIsVerifying(true);
    setVerifyResult(null);
    const finalBaseUrl = providerId === 'custom' ? customBaseUrl.trim() : activeProvider.baseUrl;
    const finalModel = selectedModel === '__custom__' ? customModelText.trim() : selectedModel;

    try {
      const res = await fetchApi<VerifyAiKeyResponse>('/ai/verify', {
        method: 'POST',
        body: JSON.stringify({
          api_base_url: finalBaseUrl || undefined,
          api_key: apiKey.trim() || undefined,
          model_name: finalModel || undefined,
        }),
      });
      setVerifyResult(res);
    } catch (err: any) {
      setVerifyResult({
        valid: false,
        message: err?.message || 'Verification failed. Unable to reach server.',
      });
    } finally {
      setIsVerifying(false);
    }
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const finalBaseUrl = providerId === 'custom' ? customBaseUrl.trim() : activeProvider.baseUrl;
    const finalModel = selectedModel === '__custom__' ? customModelText.trim() : selectedModel;

    onSave({
      api_base_url: finalBaseUrl || undefined,
      api_key: apiKey.trim() || undefined,
      model_name: finalModel || undefined,
      system_prompt: systemPrompt.trim() || undefined,
      temperature,
      max_history_turns: maxHistoryTurns,
    });
  };

  const providerOptions = AI_PROVIDERS.map((p) => ({
    value: p.id,
    label: p.name,
  }));

  const modelOptions = [
    ...activeProvider.models.map((m) => ({
      value: m.value,
      label: m.label,
    })),
    { value: '__custom__', label: 'Custom model name…' },
  ];

  return (
    <div
      className="fixed inset-0 z-50 bg-black/60 dark:bg-black/80 backdrop-blur-sm flex items-center justify-center p-4"
      role="dialog"
      aria-modal="true"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="bg-white dark:bg-neutral-950 text-neutral-900 dark:text-neutral-100 w-full max-w-lg rounded-xl border border-neutral-200 dark:border-neutral-800 shadow-2xl animate-in fade-in-0 zoom-in-95">
        <div className="p-5 border-b border-neutral-200 dark:border-neutral-800 flex items-center justify-between">
          <h2 className="text-sm font-semibold text-neutral-900 dark:text-neutral-100 flex items-center gap-2 font-sans">
            <Settings className="w-4 h-4 text-neutral-500" />
            <span>AI Assistant Settings</span>
          </h2>
          <button
            type="button"
            onClick={onClose}
            className="p-1 rounded text-neutral-400 hover:text-neutral-900 dark:hover:text-white transition cursor-pointer"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {error && (
          <div className="mx-5 mt-4 p-2.5 rounded bg-red-500/10 border border-red-500/20 text-red-600 dark:text-red-400 text-xs flex items-center gap-2 font-sans">
            <AlertCircle className="w-4 h-4 shrink-0" />
            <span>{error}</span>
          </div>
        )}

        <form onSubmit={handleSubmit} className="p-5 space-y-4">
          {/* API Endpoint Dropdown */}
          <label className="flex flex-col gap-1">
            <span className="text-[10px] font-mono uppercase tracking-wider text-neutral-500">
              API Endpoint / Provider
            </span>
            <CustomSelect
              value={providerId}
              onChange={handleProviderChange}
              options={providerOptions}
              className="w-full"
            />
            {providerId === 'custom' ? (
              <input
                type="text"
                required
                placeholder="https://api.your-provider.com/v1"
                value={customBaseUrl}
                onChange={(e) => setCustomBaseUrl(e.target.value)}
                className="mt-1 w-full h-9 px-3 bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 rounded-md text-xs font-mono text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 focus:outline-none focus:border-neutral-500"
              />
            ) : (
              <p className="text-[11px] text-neutral-500 dark:text-neutral-400 mt-0.5 font-sans">
                <span className="font-mono text-neutral-700 dark:text-neutral-300">{activeProvider.baseUrl}</span>
                {' — '}
                {activeProvider.note}
              </p>
            )}
          </label>

          {/* Model Name Dropdown */}
          <label className="flex flex-col gap-1">
            <span className="text-[10px] font-mono uppercase tracking-wider text-neutral-500">
              Model Name ({activeProvider.name})
            </span>
            <CustomSelect
              value={selectedModel}
              onChange={handleModelChange}
              options={modelOptions}
              className="w-full"
            />
            {selectedModel === '__custom__' && (
              <input
                type="text"
                required
                placeholder="e.g. gpt-4o, llama-3.3-70b, deepseek-chat"
                value={customModelText}
                onChange={(e) => setCustomModelText(e.target.value)}
                className="mt-1 w-full h-9 px-3 bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 rounded-md text-xs font-mono text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 focus:outline-none focus:border-neutral-500"
              />
            )}
          </label>

          {/* API Key */}
          <div>
            <div className="flex items-center justify-between mb-1">
              <label className="block text-[10px] font-mono uppercase tracking-wider text-neutral-500">
                API Key {config?.has_api_key && <span className="text-emerald-600 dark:text-emerald-400 font-normal ml-1">✓ Saved</span>}
              </label>
              <button
                type="button"
                onClick={handleVerify}
                disabled={isVerifying}
                className="text-[11px] font-mono text-neutral-600 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-white flex items-center gap-1 transition disabled:opacity-50 cursor-pointer"
                title="Verify API key and connection to this provider"
              >
                {isVerifying ? (
                  <>
                    <RefreshCw className="w-3 h-3 animate-spin" />
                    <span>Verifying…</span>
                  </>
                ) : (
                  <>
                    <ShieldCheck className="w-3 h-3" />
                    <span>Verify Key</span>
                  </>
                )}
              </button>
            </div>
            <input
              type="password"
              placeholder={config?.has_api_key ? '•••••••• (Enter new key to replace)' : activeProvider.apiKeyPlaceholder}
              value={apiKey}
              onChange={(e) => {
                setApiKey(e.target.value);
                if (verifyResult) setVerifyResult(null);
              }}
              className="w-full h-9 px-3 bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 rounded-md text-xs font-mono text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 focus:outline-none focus:border-neutral-500"
            />
            {verifyResult && (
              <div
                className={`mt-2 p-2.5 rounded text-xs flex items-start gap-2 font-sans ${
                  verifyResult.valid
                    ? 'bg-emerald-500/10 border border-emerald-500/20 text-emerald-700 dark:text-emerald-400'
                    : 'bg-red-500/10 border border-red-500/20 text-red-600 dark:text-red-400'
                }`}
              >
                {verifyResult.valid ? (
                  <Check className="w-4 h-4 shrink-0 mt-0.5 text-emerald-600 dark:text-emerald-400" />
                ) : (
                  <AlertCircle className="w-4 h-4 shrink-0 mt-0.5 text-red-600 dark:text-red-400" />
                )}
                <span className="leading-snug">{verifyResult.message}</span>
              </div>
            )}
          </div>

          {/* Temperature Slider */}
          <div>
            <div className="flex items-center justify-between mb-1">
              <label className="text-[10px] font-mono uppercase tracking-wider text-neutral-500">
                Temperature
              </label>
              <span className="text-xs font-mono text-neutral-700 dark:text-neutral-300">
                {temperature.toFixed(1)}
              </span>
            </div>
            <input
              type="range"
              min="0"
              max="2"
              step="0.1"
              value={temperature}
              onChange={(e) => setTemperature(parseFloat(e.target.value))}
              className="w-full mt-1 accent-neutral-900 dark:accent-neutral-100 cursor-pointer"
            />
            <div className="flex justify-between text-[10px] font-mono text-neutral-400 mt-0.5">
              <span>0.0 (Precise)</span>
              <span>1.0 (Balanced)</span>
              <span>2.0 (Creative)</span>
            </div>
          </div>

          {/* Max History Turns */}
          <div>
            <div className="flex items-center justify-between mb-1">
              <label className="text-[10px] font-mono uppercase tracking-wider text-neutral-500">
                Max Context History Messages
              </label>
              <span className="text-xs font-mono text-neutral-700 dark:text-neutral-300">
                {maxHistoryTurns} messages
              </span>
            </div>
            <input
              type="range"
              min="1"
              max="50"
              step="1"
              value={maxHistoryTurns}
              onChange={(e) => setMaxHistoryTurns(parseInt(e.target.value, 10))}
              className="w-full mt-1 accent-neutral-900 dark:accent-neutral-100 cursor-pointer"
            />
            <div className="flex justify-between text-[10px] font-mono text-neutral-400 mt-0.5">
              <span>1 (Minimal)</span>
              <span>10 (Default)</span>
              <span>50 (Deep context)</span>
            </div>
            <p className="text-[11px] font-mono text-neutral-500 mt-1">
              Number of recent conversation messages included in context when you chat or mention the AI.
            </p>
          </div>

          {/* System Prompt */}
          <div>
            <label className="block text-[10px] font-mono uppercase tracking-wider text-neutral-500 mb-1">
              Custom System Prompt (Optional)
            </label>
            <textarea
              rows={3}
              placeholder="You are an assistant who answers concisely and thoughtfully..."
              value={systemPrompt}
              onChange={(e) => setSystemPrompt(e.target.value)}
              className="w-full px-3 py-2 bg-neutral-50 dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 rounded-md text-xs text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 dark:placeholder-neutral-500 focus:outline-none focus:border-neutral-500 font-sans"
            />
          </div>

          <div className="pt-3 border-t border-neutral-200 dark:border-neutral-800 flex justify-end gap-2">
            <button
              type="button"
              onClick={onClose}
              className="h-9 px-3.5 text-xs font-medium text-neutral-700 dark:text-neutral-300 hover:text-neutral-900 dark:hover:text-neutral-100 rounded-md border border-neutral-300 dark:border-neutral-800 hover:bg-neutral-100 dark:hover:bg-neutral-900 transition cursor-pointer"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={isSaving}
              className="h-9 px-3.5 text-xs font-medium bg-neutral-900 dark:bg-neutral-100 text-white dark:text-neutral-900 hover:bg-neutral-800 dark:hover:bg-neutral-200 rounded-md transition disabled:opacity-50 cursor-pointer"
            >
              {isSaving ? 'Saving...' : 'Save Settings'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
