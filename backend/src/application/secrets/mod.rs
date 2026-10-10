//! Application service for encrypted secret credential management.
//!
//! Enforces AES-256-GCM encryption at rest, write-only APIs, audit logging,
//! and outbox event publishing.

use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::config::{ConfigAuditAction, ConfigAuditOutcome, ConfigAuditTarget};
use crate::domain::events::SystemEvent;
use crate::domain::secrets::{
    mask_secret, EncryptedSecret, SecretError, SecretId, SecretMetadata, SecretProvider,
    SecretStatus, SecretValue,
};
use crate::infrastructure::crypto::{CryptoError, SecretProtector};
use crate::infrastructure::postgres::repositories::{config_audit, outbox, provider_secrets};

/// Outcome of a credential connectivity test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResultDto {
    pub id: uuid::Uuid,
    pub status: String,
    pub error: Option<String>,
}

/// Service managing encrypted external provider credentials.
#[derive(Clone)]
pub struct SecretService {
    pool: PgPool,
    protector: Arc<dyn SecretProtector>,
}

impl SecretService {
    /// Constructs a new secret service.
    pub fn new(pool: PgPool, protector: Arc<dyn SecretProtector>) -> Self {
        Self { pool, protector }
    }

    /// Replaces the secret protector implementation.
    pub fn with_protector(&self, protector: Arc<dyn SecretProtector>) -> Self {
        Self {
            pool: self.pool.clone(),
            protector,
        }
    }

    /// Lists metadata for all stored secrets without exposing ciphertext or plaintext.
    pub async fn list_metadata(&self) -> Result<Vec<SecretMetadata>, AppError> {
        let rows = provider_secrets::list_secret_metadata(&self.pool).await?;
        let mut list = Vec::with_capacity(rows.len());

        for r in rows {
            let provider = r
                .provider
                .parse::<SecretProvider>()
                .unwrap_or(SecretProvider::Generic);
            let status = r
                .status
                .parse::<SecretStatus>()
                .unwrap_or(SecretStatus::Active);

            let masked_preview = r
                .metadata
                .as_ref()
                .and_then(|m| m.get("masked_preview"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            list.push(SecretMetadata {
                id: SecretId::from_uuid(r.id),
                provider,
                name: r.name,
                status,
                key_version: r.key_version,
                masked_preview,
                last_tested_at: r.last_tested_at,
                last_test_status: r.last_test_status,
                last_test_error: r.last_test_error,
                created_at: r.created_at,
                updated_at: r.updated_at,
                metadata: r.metadata,
            });
        }

        Ok(list)
    }

    /// Whether the underlying protector has a valid encryption key configured.
    pub fn is_configured(&self) -> bool {
        self.protector.is_configured()
    }

    /// Retrieves metadata for a single stored secret by ID.
    pub async fn get_metadata_by_id(&self, id: SecretId) -> Result<SecretMetadata, AppError> {
        let all = self.list_metadata().await?;
        all.into_iter()
            .find(|m| m.id == id)
            .ok_or_else(|| AppError::NotFound(format!("Secret not found: {id}")))
    }

    /// Stores a new encrypted credential in a single transaction with audit and outbox events.
    pub async fn store_secret(
        &self,
        actor: &AccountId,
        provider: SecretProvider,
        name: &str,
        secret: SecretValue,
        metadata: Option<serde_json::Value>,
    ) -> Result<SecretId, AppError> {
        let plain = secret.expose_secret().trim();
        if plain.is_empty() || plain.len() > 4096 {
            return Err(SecretError::InvalidPayload.into());
        }

        let trimmed_name = name.trim();
        if trimmed_name.is_empty() || trimmed_name.len() > 100 {
            return Err(SecretError::InvalidPayload.into());
        }

        let masked = mask_secret(plain);
        let mut meta = metadata.unwrap_or_else(|| serde_json::json!({}));
        if let serde_json::Value::Object(ref mut map) = meta {
            map.insert("masked_preview".to_string(), serde_json::json!(masked));
        }

        // Encrypt secret with AES-256-GCM and AAD binding
        let encrypted = self.protector.encrypt(provider, trimmed_name, &secret)?;

        let mut tx = self.pool.begin().await?;

        let row_id = provider_secrets::insert_secret(
            &mut *tx,
            provider.as_str(),
            trimmed_name,
            &encrypted.ciphertext,
            &encrypted.nonce,
            encrypted.key_version,
            Some(actor),
            Some(&meta),
        )
        .await?;

        let secret_id = SecretId::from_uuid(row_id);

        // Record non-sensitive audit entry
        let audit_detail = serde_json::json!({
            "provider": provider.as_str(),
            "name": trimmed_name,
            "masked_preview": masked,
            "key_version": encrypted.key_version,
        });
        config_audit::insert_audit_entry(
            &mut *tx,
            Some(actor),
            ConfigAuditAction::SecretCreated.as_str(),
            ConfigAuditTarget::Secret(row_id).target_type(),
            &format!("{}:{}", provider.as_str(), trimmed_name),
            ConfigAuditOutcome::Success.as_str(),
            Some(&audit_detail),
        )
        .await?;

        // Publish outbox event
        outbox::publish(
            &mut tx,
            &SystemEvent::SecretChanged {
                actor_id: *actor,
                provider: provider.as_str().to_string(),
                name: trimmed_name.to_string(),
            },
        )
        .await?;

        tx.commit().await?;

        Ok(secret_id)
    }

    /// Replaces an existing secret's ciphertext with a new secret value.
    pub async fn replace_secret(
        &self,
        actor: &AccountId,
        id: SecretId,
        secret: SecretValue,
    ) -> Result<(), AppError> {
        let plain = secret.expose_secret().trim();
        if plain.is_empty() || plain.len() > 4096 {
            return Err(SecretError::InvalidPayload.into());
        }

        let existing = provider_secrets::get_secret_by_id(&self.pool, id.uuid())
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Secret not found: {id}")))?;

        let provider = existing
            .provider
            .parse::<SecretProvider>()
            .map_err(|e| AppError::Validation(e.to_string()))?;

        let masked = mask_secret(plain);
        let mut meta = existing.metadata.unwrap_or_else(|| serde_json::json!({}));
        if let serde_json::Value::Object(ref mut map) = meta {
            map.insert("masked_preview".to_string(), serde_json::json!(masked));
        }

        let encrypted = self.protector.encrypt(provider, &existing.name, &secret)?;

        let mut tx = self.pool.begin().await?;

        provider_secrets::update_secret(
            &mut *tx,
            id.uuid(),
            &encrypted.ciphertext,
            &encrypted.nonce,
            encrypted.key_version,
            None,
            Some(actor),
            Some(&meta),
        )
        .await?;

        let audit_detail = serde_json::json!({
            "provider": provider.as_str(),
            "name": existing.name,
            "masked_preview": masked,
            "key_version": encrypted.key_version,
        });
        config_audit::insert_audit_entry(
            &mut *tx,
            Some(actor),
            ConfigAuditAction::SecretUpdated.as_str(),
            ConfigAuditTarget::Secret(id.uuid()).target_type(),
            &format!("{}:{}", provider.as_str(), existing.name),
            ConfigAuditOutcome::Success.as_str(),
            Some(&audit_detail),
        )
        .await?;

        outbox::publish(
            &mut tx,
            &SystemEvent::SecretChanged {
                actor_id: *actor,
                provider: provider.as_str().to_string(),
                name: existing.name.clone(),
            },
        )
        .await?;

        tx.commit().await?;

        Ok(())
    }

    /// Deletes an encrypted credential row.
    pub async fn delete_secret(&self, actor: &AccountId, id: SecretId) -> Result<(), AppError> {
        let existing = provider_secrets::get_secret_by_id(&self.pool, id.uuid())
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Secret not found: {id}")))?;

        let mut tx = self.pool.begin().await?;

        provider_secrets::delete_secret(&mut *tx, id.uuid()).await?;

        let audit_detail = serde_json::json!({
            "provider": existing.provider,
            "name": existing.name,
        });
        config_audit::insert_audit_entry(
            &mut *tx,
            Some(actor),
            ConfigAuditAction::SecretDeleted.as_str(),
            ConfigAuditTarget::Secret(id.uuid()).target_type(),
            &format!("{}:{}", existing.provider, existing.name),
            ConfigAuditOutcome::Success.as_str(),
            Some(&audit_detail),
        )
        .await?;

        outbox::publish(
            &mut tx,
            &SystemEvent::SecretChanged {
                actor_id: *actor,
                provider: existing.provider.clone(),
                name: existing.name.clone(),
            },
        )
        .await?;

        tx.commit().await?;

        Ok(())
    }

    /// Decrypts and returns the plaintext secret for authorized backend consumers.
    pub async fn get_decrypted_secret(
        &self,
        provider: SecretProvider,
        name: &str,
    ) -> Result<Option<SecretValue>, AppError> {
        let row =
            match provider_secrets::get_secret_by_provider(&self.pool, provider.as_str(), name)
                .await?
            {
                Some(r) => r,
                None => return Ok(None),
            };

        let encrypted = EncryptedSecret {
            ciphertext: row.secret_ciphertext,
            nonce: row.nonce,
            key_version: row.key_version,
        };

        let decrypted = self.protector.decrypt(provider, name, &encrypted)?;
        Ok(Some(decrypted))
    }

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
        let (status, error) = match self.protector.decrypt(provider, &existing.name, &encrypted) {
            Ok(secret) => {
                let test_res = match provider {
                    SecretProvider::OpenAi => {
                        crate::infrastructure::inference::testers::test_llm_credential(
                            "https://api.openai.com/v1",
                            secret.expose_secret(),
                        )
                        .await
                    }
                    SecretProvider::Anthropic => {
                        crate::infrastructure::inference::testers::test_llm_credential(
                            "https://api.anthropic.com/v1",
                            secret.expose_secret(),
                        )
                        .await
                    }
                    SecretProvider::Tavily => {
                        crate::infrastructure::inference::testers::test_tavily_credential(
                            secret.expose_secret(),
                        )
                        .await
                    }
                    SecretProvider::Telegram => {
                        crate::infrastructure::inference::testers::test_telegram_credential(
                            secret.expose_secret(),
                        )
                        .await
                    }
                    SecretProvider::Discord => {
                        crate::infrastructure::inference::testers::test_discord_credential(
                            secret.expose_secret(),
                        )
                        .await
                    }
                    SecretProvider::Email => {
                        crate::infrastructure::inference::testers::test_email_credential(
                            secret.expose_secret(),
                        )
                        .await
                    }
                    SecretProvider::Stt
                    | SecretProvider::Tts
                    | SecretProvider::TelegramWebhook
                    | SecretProvider::S3
                    | SecretProvider::Generic => Ok(()),
                };
                match test_res {
                    Ok(()) => ("healthy".to_string(), None),
                    Err(e) => ("unhealthy".to_string(), Some(e)),
                }
            }
            Err(e) => ("failed".to_string(), Some(e.to_string())),
        };

        let tested_at = Utc::now();
        provider_secrets::update_test_status(
            &self.pool,
            id.uuid(),
            &status,
            error.as_deref(),
            tested_at,
        )
        .await?;

        let audit_detail = serde_json::json!({
            "provider": provider.as_str(),
            "name": existing.name,
            "status": status,
            "error": error,
        });
        config_audit::insert_audit_entry(
            &self.pool,
            Some(actor),
            ConfigAuditAction::SecretTested.as_str(),
            ConfigAuditTarget::Secret(id.uuid()).target_type(),
            &format!("{}:{}", provider.as_str(), existing.name),
            if status == "healthy" {
                ConfigAuditOutcome::Success.as_str()
            } else {
                ConfigAuditOutcome::Failure.as_str()
            },
            Some(&audit_detail),
        )
        .await?;

        Ok(TestResultDto {
            id: id.uuid(),
            status,
            error,
        })
    }
}

impl From<CryptoError> for AppError {
    fn from(err: CryptoError) -> Self {
        AppError::External {
            service: "crypto",
            source: anyhow::Error::new(err),
        }
    }
}

impl From<SecretError> for AppError {
    fn from(err: SecretError) -> Self {
        AppError::Validation(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::crypto::{AesGcmSecretProtector, MasterKeyProvider};

    async fn setup_secret_service() -> (SecretService, AccountId) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let actor = AccountId::from_seed("admin_secrets_tester");
        crate::db::pool::seed_account(&pool, &actor).await;

        let key = [0x55u8; 32];
        let protector = Arc::new(AesGcmSecretProtector::new(MasterKeyProvider::from_raw_key(
            key,
        )));
        let service = SecretService::new(pool, protector);
        (service, actor)
    }

    #[tokio::test]
    async fn test_store_and_get_decrypted_secret() {
        let (service, actor) = setup_secret_service().await;

        let secret = SecretValue::new("sk-test-secret-value-abc123xyz");
        let id = service
            .store_secret(&actor, SecretProvider::OpenAi, "default", secret, None)
            .await
            .unwrap();

        // 1. Verify metadata returned masks preview and has no secret plaintext
        let metadata = service.list_metadata().await.unwrap();
        let found = metadata.iter().find(|m| m.id == id).expect("secret found");
        assert_eq!(found.provider, SecretProvider::OpenAi);
        assert_eq!(found.name, "default");
        assert_eq!(found.masked_preview.as_deref(), Some("sk-...3xyz"));

        // 2. Outbox event was published
        let events = outbox::event_types_for(&service.pool, &actor)
            .await
            .unwrap();
        assert!(events.contains(&"SecretChanged".to_string()));

        // 3. Audit log was recorded and contains no plaintext secret
        let audit_entries = config_audit::list_audit_entries(&service.pool, 10, 0)
            .await
            .unwrap();
        let entry = audit_entries
            .iter()
            .find(|e| e.action == "secret_created")
            .expect("audit entry found");
        assert_eq!(entry.target_key, "openai:default");
        let detail_str = entry.detail.as_ref().unwrap().to_string();
        assert!(!detail_str.contains("sk-test-secret-value-abc123xyz"));
        assert!(detail_str.contains("sk-...3xyz"));

        // 4. Decrypted secret roundtrips correctly
        let decrypted = service
            .get_decrypted_secret(SecretProvider::OpenAi, "default")
            .await
            .unwrap()
            .expect("decrypted found");
        assert_eq!(decrypted.expose_secret(), "sk-test-secret-value-abc123xyz");
    }

    #[tokio::test]
    async fn test_replace_secret() {
        let (service, actor) = setup_secret_service().await;

        let id = service
            .store_secret(
                &actor,
                SecretProvider::Tavily,
                "search_key",
                SecretValue::new("tvly-initial-key"),
                None,
            )
            .await
            .unwrap();

        service
            .replace_secret(&actor, id, SecretValue::new("tvly-updated-key"))
            .await
            .unwrap();

        let decrypted = service
            .get_decrypted_secret(SecretProvider::Tavily, "search_key")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(decrypted.expose_secret(), "tvly-updated-key");
    }

    #[tokio::test]
    async fn test_delete_secret() {
        let (service, actor) = setup_secret_service().await;

        let id = service
            .store_secret(
                &actor,
                SecretProvider::Discord,
                "bot_token",
                SecretValue::new("discord-token-to-delete"),
                None,
            )
            .await
            .unwrap();

        service.delete_secret(&actor, id).await.unwrap();

        let decrypted = service
            .get_decrypted_secret(SecretProvider::Discord, "bot_token")
            .await
            .unwrap();
        assert!(decrypted.is_none());
    }

    #[tokio::test]
    async fn test_test_credential() {
        let (service, actor) = setup_secret_service().await;

        let id = service
            .store_secret(
                &actor,
                SecretProvider::Telegram,
                "bot_token",
                SecretValue::new("123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11"),
                None,
            )
            .await
            .unwrap();

        let result = service.test_credential(&actor, id).await.unwrap();
        assert_eq!(result.status, "healthy");
        assert!(result.error.is_none());
    }
}
