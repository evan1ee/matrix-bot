//! Application service for dynamic runtime configuration management.
//!
//! Enforces domain validation, transactional persistence, audit log recording,
//! outbox event publishing, and lock-free snapshot reloading.

pub mod snapshot;

use std::sync::Arc;

use arc_swap::ArcSwap;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::config::{
    ConfigAuditAction, ConfigAuditOutcome, ConfigAuditTarget, ConfigError, SettingKey,
    SettingRegistry, SettingType, SettingValue,
};
use crate::domain::events::SystemEvent;
use crate::infrastructure::postgres::repositories::{
    app_settings, config_audit, outbox, provider_secrets,
};

pub use snapshot::RuntimeConfigSnapshot;

/// Rich description and value payload for a configuration setting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingItem {
    pub key: String,
    pub value: SettingValue,
    pub default_value: SettingValue,
    pub value_type: SettingType,
    pub description: String,
    pub is_custom: bool,
    pub hot_reloadable: bool,
    pub category: String,
    pub updated_at: Option<DateTime<Utc>>,
    pub updated_by: Option<uuid::Uuid>,
}

/// Consolidated configuration and secrets overview status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigurationStatusDto {
    pub settings_count: usize,
    pub custom_settings_count: usize,
    pub encryption_configured: bool,
    pub secrets_count: usize,
    pub active_secrets_count: usize,
    pub last_reload_at: DateTime<Utc>,
}

/// Dynamic application configuration service.
#[derive(Clone)]
pub struct AppConfigService {
    pool: PgPool,
    snapshot: Arc<ArcSwap<RuntimeConfigSnapshot>>,
    last_reload_at: Arc<ArcSwap<DateTime<Utc>>>,
}

impl AppConfigService {
    /// Creates a new service with default snapshot in memory.
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            snapshot: Arc::new(ArcSwap::from_pointee(RuntimeConfigSnapshot::default())),
            last_reload_at: Arc::new(ArcSwap::from_pointee(Utc::now())),
        }
    }

    /// Loads persisted settings from the database and constructs an initialized service.
    pub async fn load(pool: PgPool) -> Result<Self, AppError> {
        let rows = app_settings::list_settings(&pool).await?;
        let snap = RuntimeConfigSnapshot::from_persisted(&rows);
        Ok(Self {
            pool,
            snapshot: Arc::new(ArcSwap::from_pointee(snap)),
            last_reload_at: Arc::new(ArcSwap::from_pointee(Utc::now())),
        })
    }

    /// Returns a lock-free reference-counted snapshot of the active configuration.
    pub fn snapshot(&self) -> Arc<RuntimeConfigSnapshot> {
        self.snapshot.load_full()
    }

    /// Reloads the in-memory snapshot from the database.
    pub async fn reload(&self) -> Result<(), AppError> {
        let rows = app_settings::list_settings(&self.pool).await?;
        let new_snap = RuntimeConfigSnapshot::from_persisted(&rows);
        self.snapshot.store(Arc::new(new_snap));
        self.last_reload_at.store(Arc::new(Utc::now()));
        Ok(())
    }

    /// Reads a single effective setting value.
    pub async fn get_setting(&self, key: SettingKey) -> Result<SettingValue, AppError> {
        if let Some(row) = app_settings::get_setting(&self.pool, key.as_str()).await? {
            if let Ok(val) = serde_json::from_value::<SettingValue>(row.value) {
                return Ok(val);
            }
        }
        let def = SettingRegistry::definition(key);
        Ok(def.default_value.clone())
    }

    /// Lists all registered settings with current values and metadata.
    pub async fn list_settings(&self) -> Result<Vec<SettingItem>, AppError> {
        let rows = app_settings::list_settings(&self.pool).await?;
        let mut row_map = std::collections::HashMap::new();
        for r in rows {
            row_map.insert(r.key.clone(), r);
        }

        let definitions = SettingRegistry::all();
        let mut items = Vec::with_capacity(definitions.len());

        for def in definitions {
            let key_str = def.key.as_str();
            let (val, is_custom, updated_at, updated_by) =
                if let Some(row) = row_map.remove(key_str) {
                    let parsed = serde_json::from_value::<SettingValue>(row.value)
                        .unwrap_or_else(|_| def.default_value.clone());
                    (parsed, true, Some(row.updated_at), row.updated_by)
                } else {
                    (def.default_value.clone(), false, None, None)
                };

            items.push(SettingItem {
                key: key_str.to_string(),
                value: val,
                default_value: def.default_value.clone(),
                value_type: def.value_type,
                description: def.description.to_string(),
                is_custom,
                hot_reloadable: def.hot_reloadable,
                category: category_for_key(&def.key).to_string(),
                updated_at,
                updated_by,
            });
        }

        Ok(items)
    }

    /// Reads rich description and effective value for a single setting.
    pub async fn get_setting_item(&self, key: SettingKey) -> Result<SettingItem, AppError> {
        let def = SettingRegistry::definition(key);
        let key_str = def.key.as_str();
        if let Some(row) = app_settings::get_setting(&self.pool, key_str).await? {
            let parsed = serde_json::from_value::<SettingValue>(row.value)
                .unwrap_or_else(|_| def.default_value.clone());
            Ok(SettingItem {
                key: key_str.to_string(),
                value: parsed,
                default_value: def.default_value.clone(),
                value_type: def.value_type,
                description: def.description.to_string(),
                is_custom: true,
                hot_reloadable: def.hot_reloadable,
                category: category_for_key(&def.key).to_string(),
                updated_at: Some(row.updated_at),
                updated_by: row.updated_by,
            })
        } else {
            Ok(SettingItem {
                key: key_str.to_string(),
                value: def.default_value.clone(),
                default_value: def.default_value.clone(),
                value_type: def.value_type,
                description: def.description.to_string(),
                is_custom: false,
                hot_reloadable: def.hot_reloadable,
                category: category_for_key(&def.key).to_string(),
                updated_at: None,
                updated_by: None,
            })
        }
    }

    /// Fetches a paginated slice of configuration audit entries and total count.
    pub async fn list_audit_logs(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<crate::domain::config::ConfigAuditEntry>, i64), AppError> {
        let rows = config_audit::list_audit_entries(&self.pool, limit, offset).await?;
        let total = config_audit::count_audit_entries(&self.pool).await?;

        let entries = rows
            .into_iter()
            .map(|r| {
                let action = r
                    .action
                    .parse::<ConfigAuditAction>()
                    .unwrap_or(ConfigAuditAction::SettingUpdated);
                let outcome = r
                    .outcome
                    .parse::<ConfigAuditOutcome>()
                    .unwrap_or(ConfigAuditOutcome::Success);
                crate::domain::config::ConfigAuditEntry {
                    id: Some(r.id),
                    actor_id: r.actor_id.map(AccountId::new),
                    action,
                    target_type: r.target_type,
                    target_key: r.target_key,
                    outcome,
                    detail: r.detail,
                    created_at: r.created_at,
                }
            })
            .collect();

        Ok((entries, total))
    }

    /// Updates a runtime setting in a single transaction with audit and outbox events.
    pub async fn update_setting(
        &self,
        actor: &AccountId,
        key: SettingKey,
        value: SettingValue,
    ) -> Result<(), AppError> {
        // 1. Validate value through domain registry rules
        SettingRegistry::validate(key, &value)?;

        let mut tx = self.pool.begin().await?;

        // 2. Persist to app_settings table
        let json_val = serde_json::to_value(&value)
            .map_err(|e| AppError::Validation(format!("Failed to serialize setting value: {e}")))?;
        app_settings::upsert_setting(&mut *tx, key.as_str(), &json_val, Some(actor)).await?;

        // 3. Record audit entry
        let detail = serde_json::json!({
            "key": key.as_str(),
            "new_value": value,
        });
        config_audit::insert_audit_entry(
            &mut *tx,
            Some(actor),
            ConfigAuditAction::SettingUpdated.as_str(),
            ConfigAuditTarget::Setting(key.as_str().to_string()).target_type(),
            key.as_str(),
            ConfigAuditOutcome::Success.as_str(),
            Some(&detail),
        )
        .await?;

        // 4. Publish outbox event
        outbox::publish(
            &mut tx,
            &SystemEvent::ConfigUpdated {
                actor_id: *actor,
                key: key.as_str().to_string(),
            },
        )
        .await?;

        tx.commit().await?;

        // 5. Reload snapshot into lock-free atomic pointer
        self.reload().await?;

        Ok(())
    }

    /// Resets a setting back to its static default.
    pub async fn reset_setting(&self, actor: &AccountId, key: SettingKey) -> Result<(), AppError> {
        let def = SettingRegistry::definition(key);

        let mut tx = self.pool.begin().await?;

        app_settings::delete_setting(&mut *tx, key.as_str()).await?;

        let detail = serde_json::json!({
            "key": key.as_str(),
            "reset_to_default": def.default_value,
        });
        config_audit::insert_audit_entry(
            &mut *tx,
            Some(actor),
            ConfigAuditAction::SettingReset.as_str(),
            ConfigAuditTarget::Setting(key.as_str().to_string()).target_type(),
            key.as_str(),
            ConfigAuditOutcome::Success.as_str(),
            Some(&detail),
        )
        .await?;

        outbox::publish(
            &mut tx,
            &SystemEvent::ConfigUpdated {
                actor_id: *actor,
                key: key.as_str().to_string(),
            },
        )
        .await?;

        tx.commit().await?;

        self.reload().await?;

        Ok(())
    }

    /// Returns the system configuration status summary.
    pub async fn get_status(
        &self,
        encryption_configured: bool,
    ) -> Result<ConfigurationStatusDto, AppError> {
        let settings = app_settings::list_settings(&self.pool).await?;
        let custom_count = settings.len();
        let total_settings = SettingRegistry::all().len();

        let secrets_meta = provider_secrets::list_secret_metadata(&self.pool).await?;
        let secrets_count = secrets_meta.len();
        let active_secrets = secrets_meta.iter().filter(|s| s.status == "active").count();

        let last_reload = **self.last_reload_at.load();

        Ok(ConfigurationStatusDto {
            settings_count: total_settings,
            custom_settings_count: custom_count,
            encryption_configured,
            secrets_count,
            active_secrets_count: active_secrets,
            last_reload_at: last_reload,
        })
    }
}

fn category_for_key(key: &SettingKey) -> &'static str {
    let k = key.as_str();
    if k.starts_with("llm.") {
        "llm"
    } else if k.starts_with("search.") {
        "search"
    } else if k.starts_with("inference.") {
        "inference"
    } else if k.starts_with("channels.") {
        "channels"
    } else if k.starts_with("registration.") {
        "registration"
    } else {
        "general"
    }
}

impl From<ConfigError> for AppError {
    fn from(err: ConfigError) -> Self {
        match err {
            ConfigError::UnknownKey(k) => AppError::NotFound(format!("unknown setting key: {k}")),
            ConfigError::TypeMismatch {
                key,
                expected,
                actual,
            } => AppError::Validation(format!(
                "type mismatch for key '{key}': expected {expected:?}, got {actual:?}"
            )),
            ConfigError::Validation { key, reason } => {
                AppError::Validation(format!("validation failed for key '{key}': {reason}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_service() -> (AppConfigService, AccountId) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let actor = AccountId::from_seed("admin_config_tester");
        crate::db::pool::seed_account(&pool, &actor).await;
        let service = AppConfigService::load(pool).await.unwrap();
        (service, actor)
    }

    #[tokio::test]
    async fn test_get_setting_default() {
        let (service, _) = setup_service().await;
        let val = service
            .get_setting(SettingKey::LlmDefaultModel)
            .await
            .unwrap();
        assert_eq!(val, SettingValue::String("gpt-4o-mini".to_string()));
    }

    #[tokio::test]
    async fn test_update_setting_and_outbox_audit_snapshot() {
        let (service, actor) = setup_service().await;

        let new_model = SettingValue::String("claude-3-5-sonnet".to_string());
        service
            .update_setting(&actor, SettingKey::LlmDefaultModel, new_model.clone())
            .await
            .unwrap();

        // 1. Snapshot reflects change immediately
        assert_eq!(service.snapshot().llm_default_model(), "claude-3-5-sonnet");

        // 2. Database read reflects change
        let read_val = service
            .get_setting(SettingKey::LlmDefaultModel)
            .await
            .unwrap();
        assert_eq!(read_val, new_model);

        // 3. Outbox event was published
        let events = outbox::event_types_for(&service.pool, &actor)
            .await
            .unwrap();
        assert!(events.contains(&"ConfigUpdated".to_string()));

        // 4. Audit entry recorded
        let audit_entries = config_audit::list_audit_entries(&service.pool, 10, 0)
            .await
            .unwrap();
        assert!(audit_entries
            .iter()
            .any(|e| e.action == "setting_updated" && e.target_key == "llm.default_model"));
    }

    #[tokio::test]
    async fn test_update_setting_invalid_value_fails() {
        let (service, actor) = setup_service().await;

        // Temperature must be between 0.0 and 2.0
        let invalid_temp = SettingValue::Float(9.99);
        let err = service
            .update_setting(&actor, SettingKey::LlmDefaultTemperature, invalid_temp)
            .await
            .unwrap_err();

        assert!(matches!(err, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn test_reset_setting() {
        let (service, actor) = setup_service().await;

        service
            .update_setting(
                &actor,
                SettingKey::LlmDefaultProvider,
                SettingValue::String("anthropic".to_string()),
            )
            .await
            .unwrap();
        assert_eq!(service.snapshot().llm_default_provider(), "anthropic");

        service
            .reset_setting(&actor, SettingKey::LlmDefaultProvider)
            .await
            .unwrap();

        assert_eq!(service.snapshot().llm_default_provider(), "openai");
    }

    #[tokio::test]
    async fn test_list_settings_and_status() {
        let (service, _) = setup_service().await;
        let list = service.list_settings().await.unwrap();
        assert_eq!(list.len(), SettingRegistry::all().len());

        let status = service.get_status(true).await.unwrap();
        assert_eq!(status.settings_count, SettingRegistry::all().len());
    }
}

#[cfg(test)]
mod runtime_tests;
