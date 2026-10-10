//! `media.process_receipt`: processes an uploaded receipt image in object storage,
//! runs OCR text extraction, parses receipt fields, records the transaction in finance,
//! and notifies the originating conversation.

use std::sync::Arc;

use async_trait::async_trait;
use tracing::{debug, error, info, warn};

use super::{message, parse_payload, Outbound, CHANNEL_RETRY};
use crate::application::AppServices;
use crate::domain::channels::ConversationAddress;
use crate::domain::finance::receipt::extract_receipt_fields;
use crate::domain::jobs::{JobType, ProcessReceiptPayload};
use crate::domain::storage::StorageError;
use crate::infrastructure::inference::ocr::{OcrClient, OcrServiceError};
use crate::interface::channels::message::Target;
use crate::jobs::{JobContext, JobError, JobHandler};
use crate::utils::html::html_escape;

pub struct ProcessReceiptHandler {
    services: AppServices,
    outbound: Arc<dyn Outbound>,
    ocr: OcrClient,
}

impl ProcessReceiptHandler {
    pub fn new(services: AppServices, outbound: Arc<dyn Outbound>, ocr: OcrClient) -> Self {
        Self {
            services,
            outbound,
            ocr,
        }
    }

    async fn deliver_notification(
        &self,
        conversation: &Option<ConversationAddress>,
        plain: &str,
        html: &str,
    ) {
        let Some(addr) = conversation else {
            return;
        };
        let msg = message(Target::Conversation(addr.clone()), plain, html);
        if let Err(err) = self.outbound.send(msg).await {
            warn!(error = %err, "Failed to deliver receipt processing notification");
        }
    }
}

#[async_trait]
impl JobHandler for ProcessReceiptHandler {
    fn job_type(&self) -> JobType {
        JobType::MediaProcessReceipt
    }

    async fn run(&self, ctx: &JobContext, payload: serde_json::Value) -> Result<(), JobError> {
        let p: ProcessReceiptPayload = parse_payload(payload)?;
        let owner = ctx.account_id.unwrap_or(p.account_id);

        // If the originating conversation's channel is currently disconnected, defer the job.
        if let Some(addr) = &p.conversation {
            if let Some(channel) = addr.channel() {
                if !self.outbound.is_connected(channel) {
                    debug!(%channel, "Channel not connected; deferring receipt processing job");
                    return Err(JobError::Unavailable(CHANNEL_RETRY));
                }
            }
        }

        // Fetch image bytes from object storage
        let bytes = match self.services.storage.get(&p.storage_key).await {
            Ok(b) => b,
            Err(StorageError::NotFound(err)) => {
                error!(%owner, storage_key = %p.storage_key, %err, "Receipt storage object not found");
                return Err(JobError::Permanent(anyhow::anyhow!(
                    "Receipt storage object not found: {err}"
                )));
            }
            Err(StorageError::AccessDenied) => {
                error!(%owner, storage_key = %p.storage_key, "Storage access denied");
                return Err(JobError::Permanent(anyhow::anyhow!(
                    "Receipt storage access denied"
                )));
            }
            Err(err) => {
                warn!(%owner, storage_key = %p.storage_key, %err, "Transient storage retrieval error; will retry");
                return Err(JobError::Retryable(anyhow::anyhow!(
                    "Failed to fetch receipt from storage: {err}"
                )));
            }
        };

        let snapshot = self.services.app_config.snapshot();
        let ocr_disabled_by_setting = snapshot
            .is_customized(crate::domain::config::SettingKey::InferenceOcrEnabled)
            && !snapshot.inference_ocr_enabled();
        let effective_ocr = self.ocr.with_base_url_override(
            Some(snapshot.inference_ocr_base_url()).filter(|s| !s.is_empty()),
        );

        if ocr_disabled_by_setting || !effective_ocr.is_configured() {
            warn!(
                %owner,
                storage_key = %p.storage_key,
                "OCR service is not configured; saving receipt without parsing"
            );
            self.deliver_notification(
                &p.conversation,
                "OCR service is not configured. Receipt was saved but could not be parsed.",
                "⚠️ <b>OCR 服务未配置</b><br/>小票图片已保存，但未能自动解析。请手动记录支出并关联。",
            )
            .await;
            return Ok(());
        }

        // Run OCR scan
        let scan = match effective_ocr.scan(bytes.to_vec()).await {
            Ok(s) => s,
            Err(OcrServiceError::NotConfigured) => {
                warn!(%owner, "OCR service reported not configured");
                return Ok(());
            }
            Err(OcrServiceError::Upstream {
                status: 400,
                message,
            }) => {
                error!(%owner, status = 400, %message, "Uploaded image could not be decoded by OCR engine");
                self.deliver_notification(
                    &p.conversation,
                    "Receipt image could not be decoded. Please upload a clear JPG/PNG photo.",
                    "⚠️ <b>图片无法识别</b><br/>上传的图片无法解码。请重新拍摄清晰的 JPG 或 PNG 照片。",
                )
                .await;
                return Err(JobError::Permanent(anyhow::anyhow!(
                    "Undecodable receipt image: {message}"
                )));
            }
            Err(OcrServiceError::Unreachable(err)) => {
                warn!(%owner, %err, "OCR inference service unreachable; retrying");
                return Err(JobError::Retryable(anyhow::anyhow!(
                    "OCR service unreachable: {err}"
                )));
            }
            Err(OcrServiceError::Upstream { status, message }) => {
                warn!(%owner, %status, %message, "OCR inference service upstream error; retrying");
                return Err(JobError::Retryable(anyhow::anyhow!(
                    "OCR service error {status}: {message}"
                )));
            }
        };

        // Parse structured fields from recognized OCR text
        let extracted = extract_receipt_fields(&scan.text);

        let amount_opt = extracted
            .amount
            .as_deref()
            .and_then(|a| a.parse::<f64>().ok());

        if let Some(amount_f64) = amount_opt {
            let amount_minor = (amount_f64 * 100.0).round() as i64;
            if amount_minor > 0 {
                let currency = extracted.currency.as_deref().unwrap_or("USD");
                let category = extracted.category_suggestion.as_deref();
                let merchant = extracted.merchant.as_deref();

                let note = match (merchant, p.caption.as_deref()) {
                    (Some(m), Some(c)) if !c.trim().is_empty() => {
                        Some(format!("{} - {}", m.trim(), c.trim()))
                    }
                    (Some(m), _) => Some(m.trim().to_string()),
                    (None, Some(c)) if !c.trim().is_empty() => Some(c.trim().to_string()),
                    _ => None,
                };

                let room_id = p
                    .conversation
                    .as_ref()
                    .map(|c| c.to_storage())
                    .unwrap_or_else(|| "web".to_string());

                let tx = self
                    .services
                    .finance
                    .record_expense(
                        &owner,
                        amount_minor,
                        currency,
                        2,
                        category,
                        note.as_deref(),
                        Some(p.storage_key.clone()),
                        &room_id,
                        "Receipt Scanner",
                    )
                    .await?;

                info!(
                    %owner,
                    tx_id = tx.id,
                    amount_minor,
                    currency,
                    "Recorded receipt expense transaction"
                );

                let merchant_label = merchant.unwrap_or("Unknown Merchant");
                let category_label = category.unwrap_or("Uncategorized");
                let plain = format!(
                    "Receipt recorded: {} {:.2} at {} (category: {})",
                    currency, amount_f64, merchant_label, category_label
                );
                let html = format!(
                    "🧾 <b>Receipt recorded</b><br/>\
                     • <b>Merchant:</b> {}<br/>\
                     • <b>Amount:</b> {} {:.2}<br/>\
                     • <b>Category:</b> {}<br/>\
                     • <b>Transaction ID:</b> #{}",
                    html_escape(merchant_label),
                    html_escape(currency),
                    amount_f64,
                    html_escape(category_label),
                    tx.id
                );

                self.deliver_notification(&p.conversation, &plain, &html)
                    .await;
                return Ok(());
            }
        }

        // No valid amount could be detected
        info!(
            %owner,
            storage_key = %p.storage_key,
            "Could not detect positive amount in receipt OCR"
        );
        self.deliver_notification(
            &p.conversation,
            "Could not identify a total amount on the receipt image. Please enter the transaction manually.",
            "⚠️ <b>未能识别小票总金额</b><br/>图片已成功上传保存，但未能识别出有效金额。请手动在账本中记录该支出并关联小票。",
        )
        .await;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::AccountId;
    use crate::domain::channels::ConversationAddress;
    use crate::domain::storage::{StorageCategory, StorageKey};
    use crate::infrastructure::storage::local::LocalStorageService;
    use crate::infrastructure::storage::StorageService;
    use crate::interface::jobs::fake::FakeOutbound;
    use bytes::Bytes;
    use tokio_util::sync::CancellationToken;

    fn test_ctx(owner: AccountId) -> JobContext {
        JobContext {
            job_id: crate::domain::jobs::JobId(1),
            account_id: Some(owner),
            attempt: 1,
            max_attempts: 3,
            cancel: CancellationToken::new(),
        }
    }

    #[tokio::test]
    async fn malformed_payload_fails_permanently() {
        let pool = crate::db::pool::test_pool().await;
        let services = AppServices::new(&pool);
        let outbound = FakeOutbound::connected();
        let ocr = OcrClient::new(None);
        let handler = ProcessReceiptHandler::new(services, outbound, ocr);

        let ctx = test_ctx(AccountId::from_seed("acct_alice"));
        let res = handler.run(&ctx, serde_json::json!({"bogus": 123})).await;
        assert!(matches!(res, Err(JobError::Permanent(_))));
    }

    #[tokio::test]
    async fn disconnected_channel_defers_job() {
        let pool = crate::db::pool::test_pool().await;
        let services = AppServices::new(&pool);
        let outbound = FakeOutbound::disconnected();
        let ocr = OcrClient::new(None);
        let handler = ProcessReceiptHandler::new(services, outbound, ocr);

        let owner = AccountId::from_seed("acct_alice");
        let key = StorageKey::generate(&owner, StorageCategory::Receipts, "jpg");
        let payload = ProcessReceiptPayload {
            storage_key: key,
            account_id: owner,
            conversation: Some(ConversationAddress::Matrix("!room:example.org".into())),
            caption: None,
        };

        let ctx = test_ctx(owner);
        let res = handler
            .run(&ctx, serde_json::to_value(&payload).unwrap())
            .await;
        assert!(matches!(res, Err(JobError::Unavailable(_))));
    }

    #[tokio::test]
    async fn missing_storage_key_fails_permanently() {
        let pool = crate::db::pool::test_pool().await;
        let temp_dir = tempfile::tempdir().unwrap();
        let storage =
            Arc::new(LocalStorageService::new(temp_dir.path(), None, "test-secret").unwrap());
        let services = AppServices::new(&pool).with_storage(storage);
        let outbound = FakeOutbound::connected();
        let ocr = OcrClient::new(None);
        let handler = ProcessReceiptHandler::new(services, outbound, ocr);

        let owner = AccountId::from_seed("acct_alice");
        let key = StorageKey::generate(&owner, StorageCategory::Receipts, "jpg");
        let payload = ProcessReceiptPayload {
            storage_key: key,
            account_id: owner,
            conversation: None,
            caption: None,
        };

        let ctx = test_ctx(owner);
        let res = handler
            .run(&ctx, serde_json::to_value(&payload).unwrap())
            .await;
        assert!(matches!(res, Err(JobError::Permanent(_))));
    }

    #[tokio::test]
    async fn missing_ocr_config_completes_and_notifies() {
        let pool = crate::db::pool::test_pool().await;
        let temp_dir = tempfile::tempdir().unwrap();
        let storage =
            Arc::new(LocalStorageService::new(temp_dir.path(), None, "test-secret").unwrap());
        let owner = AccountId::from_seed("acct_alice");
        let key = StorageKey::generate(&owner, StorageCategory::Receipts, "jpg");

        // Put sample image bytes
        storage
            .put(&key, Bytes::from_static(b"fake image data"), "image/jpeg")
            .await
            .unwrap();

        let services = AppServices::new(&pool).with_storage(storage);
        let outbound = FakeOutbound::connected();
        let ocr = OcrClient::new(None); // Not configured
        let handler = ProcessReceiptHandler::new(services, outbound.clone(), ocr);

        let payload = ProcessReceiptPayload {
            storage_key: key,
            account_id: owner,
            conversation: Some(ConversationAddress::Matrix("!room:example.org".into())),
            caption: None,
        };

        let ctx = test_ctx(owner);
        let res = handler
            .run(&ctx, serde_json::to_value(&payload).unwrap())
            .await;
        assert!(res.is_ok());

        let sends = outbound.room_sends();
        assert_eq!(sends.len(), 1);
        assert!(sends[0].1.contains("OCR service is not configured"));
    }
}
