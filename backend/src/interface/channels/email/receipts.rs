//! Turns an emailed receipt into stored objects and OCR jobs.
//!
//! The email is only the transport: every image goes through the same
//! `media.process_receipt` job as a Matrix or Discord photo, which reads it,
//! records the expense and mails the sender the result. This step decides what
//! the sender hears *now*: what was queued and what was left out, and why.

use tracing::warn;

use super::attachments::SkippedAttachment;
use super::inbound::ParsedEmail;
use crate::application::identity::Resolution;
use crate::application::AppServices;
use crate::db::DbPool;
use crate::domain::jobs::NewJob;
use crate::domain::storage::{StorageCategory, StorageKey};
use crate::infrastructure::postgres::repositories::jobs;
use crate::interface::channels::inbound::UNLINKED_REPLY;
use crate::interface::channels::message::Reply;
use crate::utils::html::html_escape;

/// Longest caption kept; the body of a receipt email can be a whole newsletter.
const MAX_CAPTION_CHARS: usize = 500;

#[derive(Default)]
struct Tally {
    queued: usize,
    already_known: usize,
    failed: usize,
}

/// Stores and enqueues every accepted attachment of a DMARC-passed email.
///
/// Returns the reply to mail back, or `None` when there is nothing new to say:
/// the whole message had already been ingested (an IMAP re-fetch after a
/// restart), so a second email would only confuse the sender.
pub async fn ingest_receipts(
    services: &AppServices,
    pool: &DbPool,
    parsed: &ParsedEmail,
) -> Option<Reply> {
    let owner = match services.identity.resolve(&parsed.message.sender).await {
        Ok(Resolution::Account(owner)) => owner,
        Ok(Resolution::Unlinked) => return Some(Reply::html(UNLINKED_REPLY)),
        Err(err) => {
            warn!(error = %err, "could not resolve the sender of a receipt email; dropping it");
            return None;
        }
    };

    let conversation = parsed.message.conversation.clone();
    let caption = Some(parsed.message.text.trim())
        .filter(|c| !c.is_empty())
        .map(|c| c.chars().take(MAX_CAPTION_CHARS).collect::<String>());
    let message_id = parsed.message.message_id.0.as_str();

    let mut tally = Tally::default();
    for (idx, att) in parsed.attachments.iter().enumerate() {
        let key = StorageKey::generate(&owner, StorageCategory::Receipts, att.extension);
        let data = bytes::Bytes::copy_from_slice(&att.data);
        if let Err(err) = services.storage.put(&key, data, att.content_type).await {
            warn!(error = %err, "could not store an emailed receipt");
            tally.failed += 1;
            continue;
        }
        let job = NewJob::media_process_email_receipt(
            owner,
            key,
            conversation.clone(),
            caption.clone(),
            message_id,
            idx,
        );
        match jobs::enqueue(pool, &job).await {
            Ok(Some(_)) => tally.queued += 1,
            Ok(None) => tally.already_known += 1,
            Err(err) => {
                warn!(error = %err, "could not enqueue an emailed receipt");
                tally.failed += 1;
            }
        }
    }

    let silent = tally.queued == 0
        && tally.failed == 0
        && parsed.skipped_attachments.is_empty()
        && tally.already_known > 0;
    if silent {
        return None;
    }
    Some(Reply::html(summary(&tally, &parsed.skipped_attachments)))
}

fn summary(tally: &Tally, skipped: &[SkippedAttachment]) -> String {
    let mut lines = Vec::new();
    if tally.queued > 0 {
        lines.push(format!(
            "🧾 Received {} receipt(s). I will email you the result for each one.",
            tally.queued
        ));
    }
    if tally.failed > 0 {
        lines.push(format!(
            "⚠️ Could not save {} attachment(s). Please send them again.",
            tally.failed
        ));
    }
    for s in skipped {
        lines.push(format!(
            "⚠️ Skipped <b>{}</b>: {}.",
            html_escape(&s.filename),
            s.reason.describe()
        ));
    }
    lines.join("<br/>")
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use bytes::Bytes;

    use super::*;
    use crate::domain::accounts::{AccountId, STATUS_ACTIVE};
    use crate::domain::jobs::JobType;
    use crate::domain::storage::StorageError;
    use crate::infrastructure::postgres::repositories::accounts::core;
    use crate::infrastructure::postgres::repositories::identities;
    use crate::infrastructure::storage::{LocalStorageService, StorageService};
    use crate::interface::channels::email::inbound::parse_inbound_email;

    const PNG_EML: &str = include_str!("../../../../../contracts/channels/email_receipt_png.eml");

    struct BrokenStorage;

    #[async_trait::async_trait]
    impl StorageService for BrokenStorage {
        async fn put(&self, _: &StorageKey, _: Bytes, _: &str) -> Result<(), StorageError> {
            Err(StorageError::Backend("disk full".into()))
        }
        async fn get(&self, _: &StorageKey) -> Result<Bytes, StorageError> {
            Err(StorageError::AccessDenied)
        }
        async fn delete(&self, _: &StorageKey) -> Result<(), StorageError> {
            Ok(())
        }
        async fn get_presigned_url(
            &self,
            _: &StorageKey,
            _: Duration,
        ) -> Result<String, StorageError> {
            Err(StorageError::AccessDenied)
        }
    }

    fn parsed(eml: &str) -> ParsedEmail {
        parse_inbound_email(eml.as_bytes(), "mx.example.com", None).expect("fixture parses")
    }

    fn files_under(dir: &Path) -> usize {
        std::fs::read_dir(dir).map_or(0, |entries| {
            entries
                .flatten()
                .map(|e| {
                    let p = e.path();
                    if p.is_dir() {
                        files_under(&p)
                    } else {
                        1
                    }
                })
                .sum()
        })
    }

    async fn setup(
        link: bool,
    ) -> (
        AppServices,
        DbPool,
        tempfile::TempDir,
        ParsedEmail,
        AccountId,
    ) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(LocalStorageService::new(dir.path(), None, "test-secret").unwrap());
        let services = AppServices::new(&pool).with_storage(storage);
        let email = parsed(PNG_EML);
        let owner = core::create(&pool, "alice", STATUS_ACTIVE).await.unwrap();
        if link {
            identities::bind(&pool, &owner, &email.message.sender)
                .await
                .unwrap();
        }
        (services, pool, dir, email, owner)
    }

    async fn job_keys(pool: &DbPool) -> Vec<String> {
        jobs::dedupe_keys_with_type(pool, JobType::MediaProcessReceipt.as_str())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_linked_sender_gets_one_stored_object_and_one_keyed_job() {
        let (services, pool, dir, email, _) = setup(true).await;

        let reply = ingest_receipts(&services, &pool, &email).await.unwrap();

        assert!(
            reply.html_body().contains("Received 1 receipt"),
            "{}",
            reply.html_body()
        );
        assert_eq!(files_under(dir.path()), 1);
        assert_eq!(
            job_keys(&pool).await,
            vec!["email_receipt:msg-receipt-5678@example.com:0".to_owned()]
        );
    }

    #[tokio::test]
    async fn the_same_message_ingested_twice_queues_one_job_and_stays_quiet() {
        let (services, pool, _dir, email, _) = setup(true).await;

        assert!(ingest_receipts(&services, &pool, &email).await.is_some());
        assert!(ingest_receipts(&services, &pool, &email).await.is_none());

        assert_eq!(job_keys(&pool).await.len(), 1);
    }

    #[tokio::test]
    async fn an_unlinked_sender_is_refused_and_nothing_is_stored() {
        let (services, pool, dir, email, _) = setup(false).await;

        let reply = ingest_receipts(&services, &pool, &email).await.unwrap();

        assert!(reply.html_body().contains("not linked yet"));
        assert_eq!(files_under(dir.path()), 0);
        assert!(job_keys(&pool).await.is_empty());
    }

    #[tokio::test]
    async fn a_pdf_only_email_is_answered_with_the_reason() {
        let (services, pool, dir, _, _) = setup(true).await;
        let pdf = PNG_EML
            .replace("image/png", "application/pdf")
            .replace("lunch.png", "invoice.pdf")
            .replace(
                "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
                "JVBERi0xLjcK",
            );

        let reply = ingest_receipts(&services, &pool, &parsed(&pdf))
            .await
            .unwrap();

        assert!(
            reply.html_body().contains("invoice.pdf"),
            "{}",
            reply.html_body()
        );
        assert!(reply.html_body().contains("PDF"), "{}", reply.html_body());
        assert_eq!(files_under(dir.path()), 0);
        assert!(job_keys(&pool).await.is_empty());
    }

    #[tokio::test]
    async fn a_storage_failure_asks_for_a_resend_and_queues_nothing() {
        let (services, pool, _dir, email, _) = setup(true).await;
        let services = services.with_storage(Arc::new(BrokenStorage));

        let reply = ingest_receipts(&services, &pool, &email).await.unwrap();

        assert!(
            reply.html_body().contains("send them again"),
            "{}",
            reply.html_body()
        );
        assert!(job_keys(&pool).await.is_empty());
    }
}
