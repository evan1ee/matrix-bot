use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::life::DayLogInput;
use crate::domain::life::{
    normalize_mood, validate_date_str, DayActivityResponse, LifeDayLog, LifeError, LifeHorizonDay,
    LifeProfile, LifeProfileStats, LifeYearView,
};
use crate::infrastructure::postgres::read_models::life as life_read;
use crate::infrastructure::postgres::repositories::life as life_repo;
use crate::infrastructure::postgres::repositories::outbox;

impl From<LifeError> for AppError {
    fn from(err: LifeError) -> Self {
        AppError::Validation(err.to_string())
    }
}

/// Application service orchestrating life profiles, daily logs, year views, and cross-feature activity feeds.
#[derive(Clone)]
pub struct LifeService {
    pool: PgPool,
}

impl LifeService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn get_profile(&self, owner_id: &AccountId) -> Result<Option<LifeProfile>, AppError> {
        Ok(life_repo::get_profile(&self.pool, owner_id).await?)
    }

    pub async fn upsert_profile(
        &self,
        owner_id: &AccountId,
        birth_date: &str,
        expected_lifespan_years: i64,
    ) -> Result<LifeProfile, AppError> {
        validate_date_str(birth_date).map_err(|e| LifeError::InvalidDate(e.to_string()))?;
        if expected_lifespan_years <= 0 {
            return Err(LifeError::InvalidLifespan(expected_lifespan_years).into());
        }

        let mut tx = self.pool.begin().await?;
        let profile =
            life_repo::upsert_profile(&mut *tx, owner_id, birth_date, expected_lifespan_years)
                .await?;
        outbox::publish(
            &mut tx,
            &SystemEvent::LifeProfileUpdated {
                owner_id: *owner_id,
                profile: profile.clone(),
            },
        )
        .await?;
        tx.commit().await?;
        Ok(profile)
    }

    pub async fn get_day_log(
        &self,
        owner_id: &AccountId,
        log_date: &str,
    ) -> Result<Option<LifeDayLog>, AppError> {
        validate_date_str(log_date).map_err(|e| LifeError::InvalidDate(e.to_string()))?;
        Ok(life_repo::get_day_log(&self.pool, owner_id, log_date).await?)
    }

    pub async fn upsert_day_log(
        &self,
        owner_id: &AccountId,
        log_date: &str,
        input: DayLogInput<'_>,
    ) -> Result<LifeDayLog, AppError> {
        validate_date_str(log_date).map_err(|e| LifeError::InvalidDate(e.to_string()))?;
        let mood = normalize_mood(input.mood)
            .ok_or_else(|| LifeError::InvalidMood(input.mood.to_string()))?;

        let mut tx = self.pool.begin().await?;
        let log =
            life_repo::upsert_day_log(&mut *tx, owner_id, log_date, DayLogInput { mood, ..input })
                .await?;
        outbox::publish(&mut tx, &SystemEvent::LifeDayLogged(log.clone())).await?;
        tx.commit().await?;
        Ok(log)
    }

    pub async fn update_time_allocation(
        &self,
        owner_id: &AccountId,
        log_date: &str,
        time_allocation: &str,
    ) -> Result<LifeDayLog, AppError> {
        validate_date_str(log_date).map_err(|e| LifeError::InvalidDate(e.to_string()))?;

        let mut tx = self.pool.begin().await?;
        let log = life_repo::update_time_allocation(&mut *tx, owner_id, log_date, time_allocation)
            .await?;
        outbox::publish(&mut tx, &SystemEvent::LifeDayLogged(log.clone())).await?;
        tx.commit().await?;
        Ok(log)
    }

    pub async fn delete_day_log(
        &self,
        owner_id: &AccountId,
        log_date: &str,
    ) -> Result<bool, AppError> {
        validate_date_str(log_date).map_err(|e| LifeError::InvalidDate(e.to_string()))?;

        let mut tx = self.pool.begin().await?;
        let deleted = life_repo::delete_day_log(&mut *tx, owner_id, log_date).await?;
        if deleted {
            outbox::publish(
                &mut tx,
                &SystemEvent::LifeDayCleared {
                    owner_id: *owner_id,
                    date: log_date.to_string(),
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(deleted)
    }

    pub async fn get_year_view(
        &self,
        owner_id: &AccountId,
        year: i64,
    ) -> Result<LifeYearView, AppError> {
        Ok(life_repo::get_year_view(&self.pool, owner_id, year).await?)
    }

    pub async fn get_horizon_days(
        &self,
        owner_id: &AccountId,
    ) -> Result<Vec<LifeHorizonDay>, AppError> {
        let tz = crate::infrastructure::postgres::repositories::settings::get(&self.pool, owner_id)
            .await?
            .unwrap_or_default()
            .tz();
        Ok(life_read::get_horizon_days(&self.pool, owner_id, tz.name()).await?)
    }

    pub async fn get_stats(
        &self,
        owner_id: &AccountId,
        today_date_str: &str,
    ) -> Result<LifeProfileStats, AppError> {
        Ok(life_repo::get_stats(&self.pool, owner_id, today_date_str).await?)
    }

    pub async fn collect_day_activity(
        &self,
        owner_id: &AccountId,
        date: &str,
    ) -> Result<DayActivityResponse, AppError> {
        validate_date_str(date).map_err(|e| LifeError::InvalidDate(e.to_string()))?;
        let tz = crate::infrastructure::postgres::repositories::settings::get(&self.pool, owner_id)
            .await?
            .unwrap_or_default()
            .tz();
        Ok(life_read::collect_day_activity(&self.pool, owner_id, date, tz.name()).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{migrate::run_migrations_unlocked, pool::seed_accounts, pool::test_pool};
    use crate::infrastructure::postgres::repositories::outbox::event_types_for;

    #[tokio::test]
    async fn life_service_validates_inputs_manages_logs_and_publishes_outbox_events() {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;
        seed_accounts(&pool, &["life_svc_alice", "life_svc_bob"]).await;
        let alice = AccountId::from_seed("life_svc_alice");
        let bob = AccountId::from_seed("life_svc_bob");

        let svc = LifeService::new(pool.clone());

        // Validation errors on profile & day log
        assert!(matches!(
            svc.upsert_profile(&alice, "not-a-date", 80).await,
            Err(AppError::Validation(_))
        ));
        assert!(matches!(
            svc.upsert_profile(&alice, "1995-05-15", 0).await,
            Err(AppError::Validation(_))
        ));

        // Upsert profile & read back
        let profile = svc
            .upsert_profile(&alice, "1995-05-15", 85)
            .await
            .expect("upsert profile");
        assert_eq!(profile.birth_date, "1995-05-15");
        assert_eq!(profile.expected_lifespan_years, 85);
        assert_eq!(
            svc.get_profile(&alice)
                .await
                .expect("get profile")
                .expect("some")
                .expected_lifespan_years,
            85
        );
        assert!(svc.get_profile(&bob).await.expect("bob profile").is_none());

        // Invalid mood is rejected
        assert!(matches!(
            svc.upsert_day_log(
                &alice,
                "2026-10-12",
                DayLogInput {
                    mood: "unknown_mood",
                    weather: None,
                    activities: None,
                    note: None,
                    time_allocation: None,
                }
            )
            .await,
            Err(AppError::Validation(_))
        ));

        // Upsert day log with valid mood ("great" normalizes to canonical "amazing")
        let log = svc
            .upsert_day_log(
                &alice,
                "2026-10-12",
                DayLogInput {
                    mood: "great",
                    weather: Some("sunny"),
                    activities: Some("coding,running"),
                    note: Some("Shipped feature"),
                    time_allocation: None,
                },
            )
            .await
            .expect("upsert day log");
        assert_eq!(log.mood, "amazing");
        assert_eq!(log.weather.as_deref(), Some("sunny"));

        // Update time allocation
        let alloc_json = r#"["sleep","sleep","deep_work","deep_work"]"#;
        let updated = svc
            .update_time_allocation(&alice, "2026-10-12", alloc_json)
            .await
            .expect("update time allocation");
        assert_eq!(updated.time_allocation.as_deref(), Some(alloc_json));
        assert_eq!(updated.mood, "amazing");

        // Get day log, year view, stats, horizon, and day activity
        let fetched = svc
            .get_day_log(&alice, "2026-10-12")
            .await
            .expect("get_day_log")
            .expect("exists");
        assert_eq!(fetched.id, log.id);

        let year_view = svc.get_year_view(&alice, 2026).await.expect("year_view");
        assert_eq!(year_view.total_logged_days, 1);

        let stats = svc
            .get_stats(&alice, "2026-10-12")
            .await
            .expect("get_stats");
        assert_eq!(stats.total_logged_days, 1);
        assert_eq!(stats.dominant_mood.as_deref(), Some("amazing"));
        assert!(stats.age_days > 10_000);

        let horizon = svc.get_horizon_days(&alice).await.expect("horizon");
        assert_eq!(horizon.len(), 1);
        assert_eq!(horizon[0].date, "2026-10-12");

        let activity = svc
            .collect_day_activity(&alice, "2026-10-12")
            .await
            .expect("collect_day_activity");
        assert_eq!(activity.date, "2026-10-12");

        // Delete day log & verify outbox events
        assert!(!svc
            .delete_day_log(&bob, "2026-10-12")
            .await
            .expect("bob delete"));
        assert!(svc
            .delete_day_log(&alice, "2026-10-12")
            .await
            .expect("alice delete"));

        let events = event_types_for(&pool, &alice).await.expect("events");
        assert_eq!(
            events,
            vec![
                "LifeProfileUpdated",
                "LifeDayLogged",
                "LifeDayLogged",
                "LifeDayCleared"
            ]
        );
    }
}
