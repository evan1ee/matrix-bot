//! Probing a stored credential against its provider.

use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::SecretService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::config::{
    ConfigAuditAction, ConfigAuditEntry, ConfigAuditOutcome, ConfigAuditTarget,
};
use crate::domain::secrets::{EncryptedSecret, SecretId, SecretProvider};
use crate::infrastructure::postgres::repositories::{config_audit, provider_secrets};

/// Outcome of a credential connectivity test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResultDto {
    pub id: uuid::Uuid,
    pub status: String,
    pub error: Option<String>,
}

impl SecretService {
    /// Tests a stored credential's cryptographic integrity and records the test outcome.
    pub async fn test_credential(
        &self,
        actor: &AccountId,
        id: SecretId,
    ) -> Result<TestResultDto, AppError> {
        let existing = provider_secrets::get_secret_by_id(&self.pool, id.uuid())
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Secret not found: {id}")))?;

        let provider = existing
            .provider
            .parse::<SecretProvider>()
            .map_err(|e| AppError::Validation(e.to_string()))?;

        let encrypted = EncryptedSecret {
            ciphertext: existing.secret_ciphertext,
            nonce: existing.nonce,
            key_version: existing.key_version,
        };

        // Decrypt credential to verify authenticity and integrity, then test connectivity
        let (status, error, audit_outcome) =
            match self.protector.decrypt(provider, &existing.name, &encrypted) {
                Ok(secret) => {
                    let raw_secret = secret.expose_secret();
                    match self.tester.test_provider(provider, raw_secret).await {
                        Ok(Some(())) => ("healthy".to_string(), None, ConfigAuditOutcome::Success),
                        Ok(None) => ("unknown".to_string(), None, ConfigAuditOutcome::Success),
                        Err(e) => {
                            let sanitized_err = scrub_secret_from_error(&e, raw_secret);
                            (
                                "unhealthy".to_string(),
                                Some(sanitized_err),
                                ConfigAuditOutcome::Failure,
                            )
                        }
                    }
                }
                Err(_) => (
                    "failed".to_string(),
                    Some("Decryption failed (integrity or authenticity check failed)".to_string()),
                    ConfigAuditOutcome::Failure,
                ),
            };

        let tested_at = Utc::now();
        let mut tx = self.pool.begin().await?;
        provider_secrets::update_test_status(
            &mut *tx,
            id.uuid(),
            &status,
            error.as_deref(),
            tested_at,
        )
        .await?;

        let audit_entry = ConfigAuditEntry::new(
            Some(*actor),
            ConfigAuditAction::SecretTested,
            ConfigAuditTarget::Secret(id.uuid()),
            audit_outcome,
            Some(serde_json::json!({
                "provider": provider.as_str(),
                "name": existing.name,
                "status": status,
                "error": error,
            })),
        );
        config_audit::insert_audit_entry(
            &mut *tx,
            Some(actor),
            audit_entry.action.as_str(),
            &audit_entry.target_type,
            &format!("{}:{}", provider.as_str(), existing.name),
            audit_entry.outcome.as_str(),
            audit_entry.detail.as_ref(),
        )
        .await?;
        tx.commit().await?;

        Ok(TestResultDto {
            id: id.uuid(),
            status,
            error,
        })
    }
}

fn scrub_secret_from_error(err: &str, raw_secret: &str) -> String {
    let trimmed = raw_secret.trim();
    let mut out = if !raw_secret.is_empty() {
        err.replace(raw_secret, "[REDACTED]")
    } else {
        err.to_string()
    };
    if !trimmed.is_empty() {
        out = out.replace(trimmed, "[REDACTED]");
    }
    out.chars().take(500).collect()
}
