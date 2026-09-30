use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::life::DayLogInput;
use crate::domain::life::{
    normalize_mood, validate_date_str, DayActivityResponse, LifeDayLog, LifeError, LifeHorizonDay,
    LifeProfile, LifeProfileStats, LifeYearView,
};
use crate::events::EventBroadcaster;
use crate::infrastructure::postgres::read_models::life as life_read;
use crate::infrastructure::postgres::repositories::life as life_repo;

impl From<LifeError> for AppError {
    fn from(err: LifeError) -> Self {
        AppError::Validation(err.to_string())
    }
}

/// Application service orchestrating life profiles, daily logs, year views, and cross-feature activity feeds.
#[derive(Clone)]
pub struct LifeService {
    pool: PgPool,
    events: EventBroadcaster,
}

impl LifeService {
    pub fn new(pool: PgPool, events: EventBroadcaster) -> Self {
        Self { pool, events }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub fn events(&self) -> &EventBroadcaster {
        &self.events
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

        let profile =
            life_repo::upsert_profile(&self.pool, owner_id, birth_date, expected_lifespan_years)
                .await?;

        self.events.send(SystemEvent::LifeProfileUpdated {
            owner_id: *owner_id,
            profile: profile.clone(),
        });
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

        let log = life_repo::upsert_day_log(
            &self.pool,
            owner_id,
            log_date,
            DayLogInput { mood, ..input },
        )
        .await?;

        self.events.send(SystemEvent::LifeDayLogged(log.clone()));
        Ok(log)
    }

    pub async fn update_time_allocation(
        &self,
        owner_id: &AccountId,
        log_date: &str,
        time_allocation: &str,
    ) -> Result<LifeDayLog, AppError> {
        validate_date_str(log_date).map_err(|e| LifeError::InvalidDate(e.to_string()))?;

        let log =
            life_repo::update_time_allocation(&self.pool, owner_id, log_date, time_allocation)
                .await?;

        self.events.send(SystemEvent::LifeDayLogged(log.clone()));
        Ok(log)
    }

    pub async fn delete_day_log(
        &self,
        owner_id: &AccountId,
        log_date: &str,
    ) -> Result<bool, AppError> {
        validate_date_str(log_date).map_err(|e| LifeError::InvalidDate(e.to_string()))?;

        let deleted = life_repo::delete_day_log(&self.pool, owner_id, log_date).await?;
        if deleted {
            self.events.send(SystemEvent::LifeDayCleared {
                owner_id: *owner_id,
                date: log_date.to_string(),
            });
        }
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
        Ok(life_read::get_horizon_days(&self.pool, owner_id).await?)
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
        Ok(life_read::collect_day_activity(&self.pool, owner_id, date).await)
    }
}
