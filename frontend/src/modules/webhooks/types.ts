export interface WebhookItem {
  id: string;
  owner_id: string;
  name: string;
  token_prefix: string;
  default_room_id: string;
  is_active: boolean;
  total_delivered_count: number;
  failure_count: number;
  last_triggered_at?: string | null;
  created_at: string;
}

export interface WebhookOutboxItem {
  id: string;
  webhook_id: string;
  owner_id: string;
  room_id: string;
  severity: string;
  title?: string | null;
  html_body: string;
  plain_body: string;
  status: string;
  attempts: number;
  max_attempts: number;
  next_attempt_at: string;
  last_error?: string | null;
  created_at: string;
  delivered_at?: string | null;
}

export interface WebhookLogsResponse {
  items: WebhookOutboxItem[];
  total: number;
  limit: number;
  offset: number;
}

export interface CreatedWebhookResponse {
  webhook: WebhookItem;
  raw_token: string;
  example_curl_json: string;
  example_curl_telegram: string;
}

export interface JoinedRoomInfo {
  room_id: string;
  name: string;
}
