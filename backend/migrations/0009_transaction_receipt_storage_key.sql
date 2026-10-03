-- Add receipt_storage_key column to transactions table to decouple receipt image storage.
ALTER TABLE transactions ADD COLUMN IF NOT EXISTS receipt_storage_key TEXT;

CREATE INDEX IF NOT EXISTS idx_transactions_receipt_storage_key ON transactions(receipt_storage_key);
