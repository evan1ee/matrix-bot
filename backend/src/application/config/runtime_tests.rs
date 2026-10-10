use std::sync::Arc;

use crate::application::config::AppConfigService;
use crate::application::AppServices;
use crate::domain::accounts::AccountId;
use crate::domain::config::{SettingKey, SettingRegistry, SettingValue};
use crate::domain::secrets::{SecretProvider, SecretValue};
use crate::infrastructure::crypto::AesGcmSecretProtector;
use crate::llm::resolved_config::ResolvedAiConfig;
use crate::llm::search::execute_web_search_with_services;

#[test]
fn test_restart_required_settings_report_hot_reloadable_false() {
    let def = SettingRegistry::definition(SettingKey::InferenceEmbeddingModel);
    assert!(
        !def.hot_reloadable,
        "InferenceEmbeddingModel must require restart/backfill"
    );
}

#[tokio::test]
async fn test_dynamic_llm_model_hot_reload() {
    let pool = crate::db::pool::test_pool().await;
    crate::db::migrate::run_migrations_unlocked(&pool).await;
    let actor = AccountId::from_seed("admin_model_reload_tester");
    crate::db::pool::seed_account(&pool, &actor).await;

    let service = AppConfigService::load(pool.clone()).await.unwrap();
    let initial_snapshot = service.snapshot();
    assert_eq!(initial_snapshot.llm_default_model(), "gpt-4o-mini");

    let ai_env = crate::config::AiEnvConfig::default();

    // 1. Initial resolution uses snapshot/default model
    let initial_resolved = ResolvedAiConfig::resolve(&ai_env, Some(&initial_snapshot), None, None);
    assert_eq!(initial_resolved.model_name, "gpt-4o-mini");

    // 2. Update setting to a new model
    service
        .update_setting(
            &actor,
            SettingKey::LlmDefaultModel,
            SettingValue::String("claude-3-5-sonnet".to_string()),
        )
        .await
        .unwrap();

    // 3. Immediately inspect the reloaded snapshot
    let updated_snapshot = service.snapshot();
    assert_eq!(updated_snapshot.llm_default_model(), "claude-3-5-sonnet");

    // 4. Next AI resolution takes effect immediately without restart
    let updated_resolved = ResolvedAiConfig::resolve(&ai_env, Some(&updated_snapshot), None, None);
    assert_eq!(updated_resolved.model_name, "claude-3-5-sonnet");
}

#[tokio::test]
async fn test_dynamic_tavily_secret_and_toggle_hot_reload() {
    let pool = crate::db::pool::test_pool().await;
    crate::db::migrate::run_migrations_unlocked(&pool).await;
    let actor = AccountId::from_seed("admin_tavily_reload_tester");
    crate::db::pool::seed_account(&pool, &actor).await;

    let protector = Arc::new(
        AesGcmSecretProtector::from_key_strings(
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
            None,
        )
        .unwrap(),
    );

    let services = AppServices::new(&pool).with_secret_protector(protector);

    // 1. Store a test Tavily key in secrets
    services
        .secrets
        .store_secret(
            &actor,
            SecretProvider::Tavily,
            "default",
            SecretValue::new("tvly-test-dynamic-reload-key"),
            None,
        )
        .await
        .unwrap();

    // 2. Verify search uses Tavily key when enabled
    assert!(services.app_config.snapshot().search_tavily_enabled());
    let res = execute_web_search_with_services(&services, "test query").await;
    assert!(
        res.is_ok(),
        "Web search should succeed with test key: {res:?}"
    );

    // 3. Disable Tavily search dynamically
    services
        .app_config
        .update_setting(
            &actor,
            SettingKey::SearchTavilyEnabled,
            SettingValue::Boolean(false),
        )
        .await
        .unwrap();

    // 4. Verify snapshot immediately disabled Tavily
    assert!(!services.app_config.snapshot().search_tavily_enabled());
}
