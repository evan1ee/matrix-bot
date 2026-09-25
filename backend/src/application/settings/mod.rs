//! Settings use cases: reading a caller's display preferences (defaulting
//! when they have never saved any) and replacing them as a whole row.
//!
//! Read by nearly every other service and Matrix/AI adapter — see
//! `domain::settings` for why the type stays a plain data record instead of
//! a normalising constructor.

use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::settings::{RawSettingsUpdate, SettingsError, UserSettings};
use crate::events::{EventBroadcaster, SystemEvent};
use crate::infrastructure::postgres::repositories::settings as repo;

impl From<SettingsError> for AppError {
    fn from(err: SettingsError) -> Self {
        AppError::Validation(err.to_string())
    }
}

#[derive(Clone)]
pub struct SettingsService {
    pool: PgPool,
    events: EventBroadcaster,
}

impl SettingsService {
    pub fn new(pool: PgPool, events: EventBroadcaster) -> Self {
        Self { pool, events }
    }

    /// The caller's settings, or the documented defaults if they have never
    /// saved any (AGENTS.md: a missing row means defaults, never an error).
    pub async fn get(&self, owner_id: &str) -> Result<UserSettings, AppError> {
        Ok(repo::get(&self.pool, owner_id).await?.unwrap_or_default())
    }

    /// Reads settings for a caller that cannot fail its own work over a
    /// settings lookup (a command reply, another feature's read path): a
    /// database error degrades to the defaults and is logged, rather than
    /// aborting whatever the caller was actually doing.
    pub async fn get_or_default(&self, owner_id: &str) -> UserSettings {
        match self.get(owner_id).await {
            Ok(settings) => settings,
            Err(err) => {
                tracing::warn!("Falling back to default settings for {owner_id}: {err:?}");
                UserSettings::default()
            }
        }
    }

    /// Persists a whole-row replacement and announces the change.
    ///
    /// The broadcast is what keeps an open web tab honest: every timestamp it
    /// shows is formatted against these settings, so a change made over
    /// Matrix or the AI tools has to reach the SSE stream or the browser
    /// silently keeps rendering the old values.
    pub async fn save(
        &self,
        owner_id: &str,
        settings: UserSettings,
    ) -> Result<UserSettings, AppError> {
        repo::upsert(&self.pool, owner_id, &settings).await?;
        self.events.send(SystemEvent::SettingsUpdated {
            owner_id: owner_id.to_string(),
            settings: settings.clone(),
        });
        Ok(settings)
    }

    /// Validates and merges a whole-row replacement against the caller's
    /// current settings (`UserSettings::apply_update`), then saves it. The
    /// one use case behind the HTTP `PUT /api/v1/settings` endpoint.
    pub async fn update(
        &self,
        owner_id: &str,
        raw: RawSettingsUpdate,
    ) -> Result<UserSettings, AppError> {
        let current = self.get_or_default(owner_id).await;
        let updated = UserSettings::apply_update(&current, raw)?;
        self.save(owner_id, updated).await
    }

    /// The last date string `YYYY-MM-DD` the morning briefing was delivered,
    /// or `None` on a read failure — briefing delivery degrades to "not yet
    /// delivered today" rather than skipping every account because one read
    /// failed.
    pub async fn get_last_briefing_date(&self, owner_id: &str) -> Option<String> {
        repo::get_last_briefing_date(&self.pool, owner_id)
            .await
            .unwrap_or_else(|err| {
                tracing::warn!("Could not read last briefing date for {owner_id}: {err:?}");
                None
            })
    }

    pub async fn update_last_briefing_date(
        &self,
        owner_id: &str,
        date_str: &str,
    ) -> Result<(), AppError> {
        Ok(repo::update_last_briefing_date(&self.pool, owner_id, date_str).await?)
    }

    /// Subscribes to the events this service publishes. Exposed so a test
    /// outside this module can observe a broadcast without reaching into the
    /// service's private fields.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<SystemEvent> {
        self.events.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn service() -> SettingsService {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        SettingsService::new(pool, EventBroadcaster::default())
    }

    #[tokio::test]
    async fn a_caller_with_no_saved_row_gets_the_defaults() {
        let service = service().await;
        assert_eq!(
            service.get("@evan:mbm.one").await.unwrap(),
            UserSettings::default()
        );
        assert_eq!(
            service.get_or_default("@evan:mbm.one").await,
            UserSettings::default()
        );
    }

    #[tokio::test]
    async fn get_or_default_degrades_instead_of_failing_on_a_closed_pool() {
        let service = service().await;
        service.pool.close().await;

        assert_eq!(
            service.get_or_default("@evan:mbm.one").await,
            UserSettings::default()
        );
        assert!(matches!(
            service.get("@evan:mbm.one").await,
            Err(AppError::Database(_))
        ));
    }

    #[tokio::test]
    async fn save_persists_and_broadcasts_to_the_owner() {
        let service = service().await;
        let mut rx = service.events.subscribe();

        let saved = service
            .save(
                "@evan:mbm.one",
                UserSettings {
                    timezone: "Asia/Shanghai".to_string(),
                    currency: "CNY".to_string(),
                    ..UserSettings::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(saved.timezone, "Asia/Shanghai");

        let read_back = service.get("@evan:mbm.one").await.unwrap();
        assert_eq!(read_back, saved);

        let event = rx.try_recv().unwrap();
        assert_eq!(event.owner_id(), "@evan:mbm.one");
        assert!(matches!(event, SystemEvent::SettingsUpdated { .. }));
    }

    #[tokio::test]
    async fn update_validates_before_saving_anything() {
        let service = service().await;
        service
            .save(
                "@evan:mbm.one",
                UserSettings {
                    timezone: "Asia/Shanghai".to_string(),
                    ..UserSettings::default()
                },
            )
            .await
            .unwrap();

        let err = service
            .update(
                "@evan:mbm.one",
                RawSettingsUpdate {
                    timezone: "Mars/Olympus".to_string(),
                    currency: "USD".to_string(),
                    dev_room_id: None,
                    max_history_turns: None,
                    briefing_enabled: None,
                    briefing_time: None,
                    tts_voice: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));

        // Rejected input must not have clobbered the previously saved value.
        let unchanged = service.get("@evan:mbm.one").await.unwrap();
        assert_eq!(unchanged.timezone, "Asia/Shanghai");
    }

    #[tokio::test]
    async fn update_merges_with_the_current_row() {
        let service = service().await;
        service
            .save(
                "@evan:mbm.one",
                UserSettings {
                    max_history_turns: 42,
                    ..UserSettings::default()
                },
            )
            .await
            .unwrap();

        let updated = service
            .update(
                "@evan:mbm.one",
                RawSettingsUpdate {
                    timezone: "Asia/Tokyo".to_string(),
                    currency: "JPY".to_string(),
                    dev_room_id: None,
                    max_history_turns: None,
                    briefing_enabled: None,
                    briefing_time: None,
                    tts_voice: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.timezone, "Asia/Tokyo");
        assert_eq!(
            updated.max_history_turns, 42,
            "an omitted field must survive the update"
        );
    }

    #[tokio::test]
    async fn last_briefing_date_round_trips() {
        let service = service().await;
        let owner = "@evan:mbm.one";
        assert_eq!(service.get_last_briefing_date(owner).await, None);

        service
            .update_last_briefing_date(owner, "2026-09-18")
            .await
            .unwrap();

        assert_eq!(
            service.get_last_briefing_date(owner).await,
            Some("2026-09-18".to_string())
        );
    }
}
