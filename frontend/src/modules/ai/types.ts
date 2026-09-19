import type { components } from '../../types/api';

export type AiUserConfig = components['schemas']['AiUserConfig'];

export type AiConversation = components['schemas']['AiConversation'];

// Server types `role` as a bare `string` (the Rust enum has no utoipa enum
// annotation), but the API only ever produces these four values. Narrowed
// here rather than widened, so `m.role === 'user'` checks stay meaningful.
// Backend finding: AiMessage.role should be a proper OpenAPI enum.
export type AiMessage = Omit<components['schemas']['AiMessage'], 'role'> & {
  role: 'system' | 'user' | 'assistant' | 'tool';
};

export type AiChatResponse = components['schemas']['AiChatResponse'];

export type CurrencySpendSummary = components['schemas']['CurrencySpendSummary'];

export type UpcomingBillsSummary = components['schemas']['UpcomingBillsSummary'];

export type WeeklyRetrospective = components['schemas']['WeeklyRetrospective'];

export type VerifyAiKeyResponse = components['schemas']['VerifyAiKeyResponse'];
