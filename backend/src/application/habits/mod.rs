//! Application service for habit tracking routines and check-in logs.
//!
//! Orchestrates habit CRUD, check-in toggles, streak computation, and event publication.

use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::habits::{
    Habit, HabitStatsSummary, HabitWithStatus, HeatmapDay, MAX_HABIT_NAME_LEN,
};
use crate::domain::habits::{HabitFields, HabitUpdate};
use crate::infrastructure::postgres::repositories::{self, outbox};

#[derive(Clone)]
pub struct HabitService {
    pool: PgPool,
}

impl HabitService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn list_with_status(
        &self,
        owner: &AccountId,
        today_str: &str,
    ) -> Result<Vec<HabitWithStatus>, AppError> {
        let items =
            repositories::habits::find_all_with_status(&self.pool, owner, today_str).await?;
        Ok(items)
    }

    pub async fn find_by_id(&self, owner: &AccountId, id: i64) -> Result<Option<Habit>, AppError> {
        let item = repositories::habits::find_by_id(&self.pool, id, owner).await?;
        Ok(item)
    }

    pub async fn find_by_name(
        &self,
        owner: &AccountId,
        name: &str,
    ) -> Result<Option<Habit>, AppError> {
        let item = repositories::habits::find_by_name(&self.pool, name, owner).await?;
        Ok(item)
    }

    pub async fn create_habit(
        &self,
        owner: &AccountId,
        name: &str,
        description: Option<&str>,
        frequency: Option<&str>,
        category: Option<&str>,
        target_days_per_week: Option<i64>,
    ) -> Result<Habit, AppError> {
        let trimmed_name = name.trim();
        if trimmed_name.is_empty() {
            return Err(AppError::Validation("Habit name is required.".to_string()));
        }
        if trimmed_name.len() > MAX_HABIT_NAME_LEN {
            return Err(AppError::Validation(
                "Habit name must be 100 characters or fewer.".to_string(),
            ));
        }

        let frequency = frequency.unwrap_or("daily");
        let category = category.unwrap_or("wellness");
        let target_days = target_days_per_week.unwrap_or(7).clamp(1, 7);

        let mut tx = self.pool.begin().await?;
        let habit = repositories::habits::create_habit(
            &mut *tx,
            owner,
            trimmed_name,
            description,
            frequency,
            category,
            target_days,
        )
        .await?;

        outbox::publish(&mut tx, &SystemEvent::HabitCreated(habit.clone())).await?;
        tx.commit().await?;
        Ok(habit)
    }

    pub async fn update_habit(
        &self,
        owner: &AccountId,
        id: i64,
        update: HabitUpdate<'_>,
    ) -> Result<Habit, AppError> {
        let trimmed_name = update.name.trim();
        if trimmed_name.is_empty() {
            return Err(AppError::Validation(
                "Habit name cannot be empty.".to_string(),
            ));
        }
        if trimmed_name.len() > MAX_HABIT_NAME_LEN {
            return Err(AppError::Validation(
                "Habit name must be 100 characters or fewer.".to_string(),
            ));
        }

        let mut tx = self.pool.begin().await?;
        let updated = repositories::habits::update_habit(
            &mut *tx,
            id,
            owner,
            HabitFields {
                name: trimmed_name,
                description: update.description,
                frequency: update.frequency.unwrap_or("daily"),
                category: update.category.unwrap_or("wellness"),
                target_days_per_week: update.target_days_per_week.unwrap_or(7).clamp(1, 7),
            },
        )
        .await?;

        let habit = updated.ok_or_else(|| AppError::NotFound(format!("Habit #{id} not found")))?;
        outbox::publish(&mut tx, &SystemEvent::HabitUpdated(habit.clone())).await?;
        tx.commit().await?;
        Ok(habit)
    }

    pub async fn delete_habit(&self, owner: &AccountId, id: i64) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;
        let deleted = repositories::habits::delete_habit(&mut *tx, id, owner).await?;
        if !deleted {
            return Err(AppError::NotFound(format!("Habit #{id} not found")));
        }
        outbox::publish(
            &mut tx,
            &SystemEvent::HabitDeleted {
                id,
                owner_id: *owner,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn toggle_log(
        &self,
        owner: &AccountId,
        id: i64,
        log_date: &str,
        note: Option<&str>,
    ) -> Result<bool, AppError> {
        let habit = repositories::habits::find_by_id(&self.pool, id, owner)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Habit #{id} not found")))?;

        let mut tx = self.pool.begin().await?;
        let completed =
            repositories::habits::toggle_log(&mut tx, habit.id, owner, log_date, note).await?;
        outbox::publish(
            &mut tx,
            &SystemEvent::HabitLogToggled {
                habit_id: habit.id,
                owner_id: *owner,
                log_date: log_date.to_string(),
                completed,
            },
        )
        .await?;
        tx.commit().await?;

        Ok(completed)
    }

    pub async fn get_heatmap(&self, owner: &AccountId) -> Result<Vec<HeatmapDay>, AppError> {
        let heatmap = repositories::habits::get_heatmap(&self.pool, owner).await?;
        Ok(heatmap)
    }

    pub async fn get_summary_stats(
        &self,
        owner: &AccountId,
        today_str: &str,
    ) -> Result<HabitStatsSummary, AppError> {
        let habits = self.list_with_status(owner, today_str).await?;
        Ok(HabitStatsSummary::from_habits(&habits))
    }
}

#[cfg(test)]
mod tests {
    use crate::application::AppServices;
    use crate::domain::accounts::AccountId;
    use crate::infrastructure::postgres::repositories;

    async fn setup(seed: &str) -> (sqlx::PgPool, AppServices, AccountId) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed(seed);
        crate::db::pool::seed_account(&pool, &owner).await;
        let services = AppServices::new(&pool);
        (pool, services, owner)
    }

    /// Given a habit, When a check-in is toggled, Then HabitLogToggled is in the outbox.
    #[tokio::test]
    async fn toggle_log_writes_habit_log_toggled_to_the_outbox() {
        let (pool, services, owner) = setup("habit_outbox_toggle").await;
        let habit = services
            .habits
            .create_habit(&owner, "Read", None, None, None, None)
            .await
            .unwrap();

        assert!(services
            .habits
            .toggle_log(&owner, habit.id, "2026-09-30", None)
            .await
            .unwrap());

        assert_eq!(
            repositories::outbox::event_types_for(&pool, &owner)
                .await
                .unwrap(),
            vec!["HabitCreated", "HabitLogToggled"]
        );
    }

    /// Given a name PostgreSQL rejects (NUL byte), When create fails, Then no
    /// habit row and no event exists.
    #[tokio::test]
    async fn a_failed_create_leaves_no_row_and_no_event() {
        let (pool, services, owner) = setup("habit_outbox_atomic").await;

        assert!(services
            .habits
            .create_habit(&owner, "a\0b", None, None, None, None)
            .await
            .is_err());

        assert!(services
            .habits
            .find_by_name(&owner, "a")
            .await
            .unwrap()
            .is_none());
        assert!(repositories::outbox::event_types_for(&pool, &owner)
            .await
            .unwrap()
            .is_empty());
    }
}
