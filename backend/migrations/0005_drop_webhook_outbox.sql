-- Step 9: webhook alerts now go through outbox_events/outbox_deliveries.
-- History is deliberately not migrated (spec O2, user decision).
DROP TABLE webhook_outbox;
