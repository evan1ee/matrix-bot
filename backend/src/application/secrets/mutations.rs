//! Writes to the vault: store, replace and delete, each audited and
//! published in the same transaction as the change.

use super::SecretService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::config::{
    ConfigAuditAction, ConfigAuditEntry, ConfigAuditOutcome, ConfigAuditTarget,
};
use crate::domain::events::SystemEvent;
use crate::domain::secrets::{mask_secret, SecretError, SecretId, SecretProvider, SecretValue};
use crate::infrastructure::postgres::repositories::{config_audit, outbox, provider_secrets};

impl SecretService {
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
        let trimmed_secret = SecretValue::new(plain);

        let trimmed_name = name.trim();
        if trimmed_name.is_empty() || trimmed_name.len() > 100 {
            return Err(SecretError::InvalidPayload.into());
        }

        let masked = mask_secret(plain);
        let mut meta = metadata.unwrap_or_else(|| serde_json::json!({}));
        if let serde_json::Value::Object(ref mut map) = meta {
            map.insert("masked_preview".to_string(), serde_json::json!(masked));
        }

        // Encrypt trimmed secret with AES-256-GCM and AAD binding
        let encrypted = self
            .protector
            .encrypt(provider, trimmed_name, &trimmed_secret)?;

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
        .await
        .map_err(|e| {
            if e.as_database_error().and_then(|db| db.code()).as_deref() == Some("23505") {
                AppError::Conflict(format!(
                    "A secret for provider '{}' and slot '{}' already exists",
                    provider.as_str(),
                    trimmed_name
                ))
            } else {
                AppError::Database(e)
            }
        })?;

        let secret_id = SecretId::from_uuid(row_id);

        // Record sanitized audit entry
        let audit_entry = ConfigAuditEntry::new(
            Some(*actor),
            ConfigAuditAction::SecretCreated,
            ConfigAuditTarget::Secret(row_id),
            ConfigAuditOutcome::Success,
            Some(serde_json::json!({
                "provider": provider.as_str(),
                "name": trimmed_name,
                "masked_preview": masked,
                "key_version": encrypted.key_version,
            })),
        );
        config_audit::insert_audit_entry(
            &mut *tx,
            Some(actor),
            audit_entry.action.as_str(),
            &audit_entry.target_type,
            &format!("{}:{}", provider.as_str(), trimmed_name),
            audit_entry.outcome.as_str(),
            audit_entry.detail.as_ref(),
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
        let trimmed_secret = SecretValue::new(plain);

        let mut tx = self.pool.begin().await?;

        let existing = provider_secrets::get_secret_by_id(&mut *tx, id.uuid())
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

        let encrypted = self
            .protector
            .encrypt(provider, &existing.name, &trimmed_secret)?;

        let updated = provider_secrets::update_secret(
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
        if !updated {
            return Err(AppError::NotFound(format!("Secret not found: {id}")));
        }

        let audit_entry = ConfigAuditEntry::new(
            Some(*actor),
            ConfigAuditAction::SecretUpdated,
            ConfigAuditTarget::Secret(id.uuid()),
            ConfigAuditOutcome::Success,
            Some(serde_json::json!({
                "provider": provider.as_str(),
                "name": existing.name,
                "masked_preview": masked,
                "key_version": encrypted.key_version,
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
        let mut tx = self.pool.begin().await?;

        let existing = provider_secrets::get_secret_by_id(&mut *tx, id.uuid())
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Secret not found: {id}")))?;

        let deleted = provider_secrets::delete_secret(&mut *tx, id.uuid()).await?;
        if !deleted {
            return Err(AppError::NotFound(format!("Secret not found: {id}")));
        }

        let audit_entry = ConfigAuditEntry::new(
            Some(*actor),
            ConfigAuditAction::SecretDeleted,
            ConfigAuditTarget::Secret(id.uuid()),
            ConfigAuditOutcome::Success,
            Some(serde_json::json!({
                "provider": existing.provider,
                "name": existing.name,
            })),
        );
        config_audit::insert_audit_entry(
            &mut *tx,
            Some(actor),
            audit_entry.action.as_str(),
            &audit_entry.target_type,
            &format!("{}:{}", existing.provider, existing.name),
            audit_entry.outcome.as_str(),
            audit_entry.detail.as_ref(),
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
}
