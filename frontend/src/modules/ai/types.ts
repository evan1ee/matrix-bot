export interface AiUserConfig {
  owner_id: string;
  api_base_url?: string | null;
  api_key?: string | null;
  model_name?: string | null;
  system_prompt?: string | null;
  temperature?: number | null;
  max_tokens?: number | null;
  updated_at: string;
  has_api_key?: boolean;
}

export interface AiConversation {
  id: string;
  owner_id: string;
  room_id: string;
  created_at: string;
  updated_at: string;
}

export interface AiMessage {
  id: string;
  conversation_id: string;
  role: 'system' | 'user' | 'assistant' | 'tool';
  content: string;
  tool_call_id?: string | null;
  tool_name?: string | null;
  created_at: string;
}

export interface AiChatResponse {
  conversation_id: string;
  message: string;
  model_used: string;
  tool_calls_executed: string[];
}
