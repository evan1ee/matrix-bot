export interface AiModelOption {
  value: string;
  label: string;
}

export interface AiProviderPreset {
  id: string;
  name: string;
  baseUrl: string;
  defaultModel: string;
  models: AiModelOption[];
  requiresApiKey: boolean;
  apiKeyPlaceholder: string;
  note: string;
}

export const AI_PROVIDERS: AiProviderPreset[] = [
  {
    id: 'openai',
    name: 'OpenAI (Official)',
    baseUrl: 'https://api.openai.com/v1',
    defaultModel: 'gpt-4o-mini',
    models: [
      { value: 'gpt-6-astra', label: 'gpt-6-astra — Flagship model for complex reasoning & coding' },
      { value: 'gpt-5.6-sol', label: 'gpt-5.6-sol — Flagship model for complex professional work' },
      { value: 'gpt-5.6-terra', label: 'gpt-5.6-terra — Balanced intelligence and cost' },
      { value: 'gpt-5.6-luna', label: 'gpt-5.6-luna — Fast & cost-sensitive high-volume' },
      { value: 'gpt-4o-mini', label: 'gpt-4o-mini — Fast, intelligent, cost-efficient (Default)' },
      { value: 'gpt-4o', label: 'gpt-4o — Flagship high-intelligence omni model' },
      { value: 'o3-mini', label: 'o3-mini — Advanced reasoning, STEM & coding' },
      { value: 'o3', label: 'o3 — High-compute deep reasoning' },
      { value: 'o1', label: 'o1 — Deep reasoning & reflective problem solving' },
      { value: 'o1-mini', label: 'o1-mini — Lightweight fast reasoning' },
      { value: 'chatgpt-4o-latest', label: 'chatgpt-4o-latest — Dynamic model matching ChatGPT web' },
    ],
    requiresApiKey: true,
    apiKeyPlaceholder: 'sk-proj-...',
    note: 'Standard OpenAI API. Requires an OpenAI API key.',
  },
  {
    id: 'xai',
    name: 'xAI / Grok',
    baseUrl: 'https://api.x.ai/v1',
    defaultModel: 'grok-4.6',
    models: [
      { value: 'grok-4.6', label: 'grok-4.6 — Frontier reasoning, coding & agentic tasks' },
      { value: 'grok-4.3', label: 'grok-4.3 — Fast & efficient reasoning' },
      { value: 'grok-3', label: 'grok-3 — High-intelligence reasoning' },
      { value: 'grok-2-1212', label: 'grok-2-1212 — Grok 2 flagship' },
      { value: 'grok-2-vision-1212', label: 'grok-2-vision-1212 — Grok 2 multimodal vision' },
      { value: 'grok-beta', label: 'grok-beta — Grok Beta' },
    ],
    requiresApiKey: true,
    apiKeyPlaceholder: 'xai-...',
    note: 'Standard xAI API. Requires an xAI API key from console.x.ai.',
  },
  {
    id: 'lm_studio',
    name: 'LM Studio (Localhost :1234)',
    baseUrl: 'http://localhost:1234/v1',
    defaultModel: 'local-model',
    models: [
      { value: 'local-model', label: 'local-model — Currently loaded model in LM Studio' },
      { value: 'deepseek-r1-distill-qwen-32b', label: 'deepseek-r1-distill-qwen-32b — Reasoning' },
      { value: 'deepseek-r1-distill-llama-8b', label: 'deepseek-r1-distill-llama-8b — Lightweight' },
      { value: 'qwen2.5-coder-32b-instruct', label: 'qwen2.5-coder-32b-instruct — Coding specialist' },
      { value: 'qwen2.5-7b-instruct', label: 'qwen2.5-7b-instruct — Fast general' },
      { value: 'llama-3.3-70b-instruct', label: 'llama-3.3-70b-instruct — 70B Flagship' },
      { value: 'llama-3.2-3b-instruct', label: 'llama-3.2-3b-instruct — Ultra-fast' },
      { value: 'phi-4', label: 'phi-4 — Microsoft 14B' },
      { value: 'mistral-nemo-instruct-2407', label: 'mistral-nemo-instruct-2407 — 12B' },
    ],
    requiresApiKey: false,
    apiKeyPlaceholder: 'Optional (LM Studio does not require an API key)',
    note: 'Local server on port 1234. Start the server in LM Studio Local Server tab.',
  },
  {
    id: 'ollama',
    name: 'Ollama (Localhost :11434)',
    baseUrl: 'http://localhost:11434/v1',
    defaultModel: 'llama3.3',
    models: [
      { value: 'llama3.3', label: 'llama3.3 — Meta Llama 3.3 70B' },
      { value: 'llama3.2', label: 'llama3.2 — Lightweight 3B/1B' },
      { value: 'deepseek-r1', label: 'deepseek-r1 — DeepSeek R1 Reasoning' },
      { value: 'deepseek-r1:8b', label: 'deepseek-r1:8b — 8B Reasoning' },
      { value: 'deepseek-r1:14b', label: 'deepseek-r1:14b — 14B Reasoning' },
      { value: 'qwen2.5-coder', label: 'qwen2.5-coder — Code generation' },
      { value: 'qwen2.5', label: 'qwen2.5 — General intelligence' },
      { value: 'phi4', label: 'phi4 — Microsoft Phi-4 (14B)' },
      { value: 'mistral', label: 'mistral — 7B versatile' },
      { value: 'gemma2', label: 'gemma2 — Google Gemma 2 (9B/27B)' },
    ],
    requiresApiKey: false,
    apiKeyPlaceholder: 'Optional (not required for local Ollama)',
    note: 'Runs offline via Ollama. API key is optional.',
  },
  {
    id: 'deepseek',
    name: 'DeepSeek',
    baseUrl: 'https://api.deepseek.com/v1',
    defaultModel: 'deepseek-chat',
    models: [
      { value: 'deepseek-chat', label: 'deepseek-chat — DeepSeek-V3 General Chat' },
      { value: 'deepseek-reasoner', label: 'deepseek-reasoner — DeepSeek-R1 Chain-of-Thought Reasoning' },
      { value: 'deepseek-v4-flash', label: 'deepseek-v4-flash — DeepSeek V4 Flash' },
      { value: 'deepseek-v4-pro', label: 'deepseek-v4-pro — DeepSeek V4 Pro' },
    ],
    requiresApiKey: true,
    apiKeyPlaceholder: 'sk-...',
    note: 'High-performance DeepSeek API. Requires DeepSeek API key.',
  },
  {
    id: 'groq',
    name: 'Groq Cloud',
    baseUrl: 'https://api.groq.com/openai/v1',
    defaultModel: 'llama-3.3-70b-versatile',
    models: [
      { value: 'llama-3.3-70b-versatile', label: 'llama-3.3-70b-versatile — Fast & powerful 70B' },
      { value: 'llama-3.1-8b-instant', label: 'llama-3.1-8b-instant — Ultra-low latency 8B' },
      { value: 'deepseek-r1-distill-llama-70b', label: 'deepseek-r1-distill-llama-70b — R1 on Groq LPU' },
      { value: 'qwen-2.5-32b', label: 'qwen-2.5-32b — High speed Qwen' },
      { value: 'mixtral-8x7b-32768', label: 'mixtral-8x7b-32768 — 32k context' },
      { value: 'gemma2-9b-it', label: 'gemma2-9b-it — Google Gemma 2 9B' },
    ],
    requiresApiKey: true,
    apiKeyPlaceholder: 'gsk_...',
    note: 'Ultra-fast LPU inference engine. Requires Groq API key.',
  },
  {
    id: 'openrouter',
    name: 'OpenRouter',
    baseUrl: 'https://openrouter.ai/api/v1',
    defaultModel: 'anthropic/claude-3.7-sonnet',
    models: [
      { value: 'openai/gpt-6-astra', label: 'openai/gpt-6-astra — Flagship reasoning & coding' },
      { value: 'openai/gpt-5.6-sol', label: 'openai/gpt-5.6-sol — Flagship professional' },
      { value: 'openai/gpt-5.6-terra', label: 'openai/gpt-5.6-terra — Balanced intelligence & cost' },
      { value: 'openai/gpt-4o-mini', label: 'openai/gpt-4o-mini — Fast & efficient' },
      { value: 'openai/gpt-4o', label: 'openai/gpt-4o — Omni flagship' },
      { value: 'openai/o3-mini', label: 'openai/o3-mini — Advanced reasoning' },
      { value: 'x-ai/grok-4.6', label: 'x-ai/grok-4.6 — xAI Grok 4.6' },
      { value: 'x-ai/grok-3', label: 'x-ai/grok-3 — xAI Grok 3' },
      { value: 'anthropic/claude-3.7-sonnet', label: 'anthropic/claude-3.7-sonnet — Hybrid reasoning & coding' },
      { value: 'anthropic/claude-3.5-sonnet', label: 'anthropic/claude-3.5-sonnet — High intelligence' },
      { value: 'anthropic/claude-3.5-haiku', label: 'anthropic/claude-3.5-haiku — Ultra-fast Claude' },
      { value: 'deepseek/deepseek-r1', label: 'deepseek/deepseek-r1 — Full 671B reasoning' },
      { value: 'deepseek/deepseek-chat', label: 'deepseek/deepseek-chat — DeepSeek V3' },
      { value: 'google/gemini-2.0-flash-001', label: 'google/gemini-2.0-flash-001' },
      { value: 'meta-llama/llama-3.3-70b-instruct', label: 'meta-llama/llama-3.3-70b-instruct' },
    ],
    requiresApiKey: true,
    apiKeyPlaceholder: 'sk-or-v1-...',
    note: 'Universal API gateway for 200+ models. Requires an OpenRouter API key.',
  },
  {
    id: 'local_8000',
    name: 'vLLM / LocalAI (Port 8000)',
    baseUrl: 'http://localhost:8000/v1',
    defaultModel: 'default',
    models: [
      { value: 'default', label: 'default — Currently served model' },
      { value: 'meta-llama/Llama-3.3-70B-Instruct', label: 'meta-llama/Llama-3.3-70B-Instruct' },
      { value: 'Qwen/Qwen2.5-Coder-32B-Instruct', label: 'Qwen/Qwen2.5-Coder-32B-Instruct' },
    ],
    requiresApiKey: false,
    apiKeyPlaceholder: 'Optional for local server',
    note: 'Local vLLM, FastChat, or LocalAI inference server.',
  },
  {
    id: 'custom',
    name: 'Custom Endpoint...',
    baseUrl: '',
    defaultModel: '',
    models: [],
    requiresApiKey: true,
    apiKeyPlaceholder: 'API key if required by endpoint',
    note: 'OpenAI-compatible custom URL (e.g. self-hosted, Azure, vLLM).',
  },
];

export function normalizeUrl(url?: string | null): string {
  if (!url) return '';
  return url.trim().replace(/\/+$/, '').toLowerCase();
}

export function findProviderByUrl(url?: string | null): AiProviderPreset {
  if (!url || !url.trim()) return AI_PROVIDERS[0]; // OpenAI
  const norm = normalizeUrl(url);
  const found = AI_PROVIDERS.find((p) => p.baseUrl && normalizeUrl(p.baseUrl) === norm);
  if (found) return found;
  return AI_PROVIDERS.find((p) => p.id === 'custom') || AI_PROVIDERS[0];
}
