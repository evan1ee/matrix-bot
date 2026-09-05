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

export interface WeeklyRetrospective {
  period_start: string;
  period_end: string;
  total_spent_minor: number;
  total_income_minor: number;
  net_cashflow_minor: number;
  currency: string;
  top_expense_category?: string | null;
  completed_tasks_count: number;
  active_tasks_count: number;
  habits_adherence_percent: number;
  active_habits_count: number;
  upcoming_bills_count: number;
  upcoming_bills_total_minor: number;
  upcoming_reminders_count: number;
  days_logged_count: number;
  dominant_mood?: string | null;
  formatted_markdown: string;
}

