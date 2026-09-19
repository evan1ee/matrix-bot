import type { components } from '../../types/api';

// Frontend-only name; server schema is Webhook.
export type WebhookItem = components['schemas']['Webhook'];

export type WebhookOutboxItem = components['schemas']['WebhookOutboxItem'];

export type WebhookLogsResponse = components['schemas']['WebhookLogsResponse'];

export type CreatedWebhookResponse = components['schemas']['CreatedWebhookResponse'];

export type JoinedRoomInfo = components['schemas']['JoinedRoomInfo'];
