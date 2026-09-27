//! Application service for habit tracking routines and check-in logs.
//!
//! Orchestrates habit CRUD, check-in toggles, streak computation, and event publication.

use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::habits::{
    Habit, HabitStatsSummary, HabitWithStatus, HeatmapDay, MAX_HABIT_NAME_LEN,
};
use crate::events::{EventBroadcaster, SystemEvent};
use crate::infrastructure::postgres::repositories;

#[derive(Clone)]
pub struct HabitService {
    pool: PgPool,
    events: EventBroadcaster,
}

impl HabitService {
    pub fn new(pool: PgPool, events: EventBroadcaster) -> Self {
        Self { pool, events }
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

        let habit = repositories::habits::create_habit(
            &self.pool,
            owner,
            trimmed_name,
            description,
            frequency,
            category,
            target_days,
        )
        .await?;

        self.events.send(SystemEvent::HabitCreated(habit.clone()));
        Ok(habit)
    }

    pub async fn update_habit(
        &self,
        owner: &AccountId,
        id: i64,
        name: &str,
        description: Option<&str>,
        frequency: Option<&str>,
        category: Option<&str>,
        target_days_per_week: Option<i64>,
    ) -> Result<Habit, AppError> {
        let trimmed_name = name.trim();
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

        let frequency = frequency.unwrap_or("daily");
        let category = category.unwrap_or("wellness");
        let target_days = target_days_per_week.unwrap_or(7).clamp(1, 7);

        let updated = repositories::habits::update_habit(
            &self.pool,
            id,
            owner,
            trimmed_name,
            description,
            frequency,
            category,
            target_days,
        )
        .await?;

        let habit = updated.ok_or_else(|| AppError::NotFound(format!("Habit #{id} not found")))?;
        self.events.send(SystemEvent::HabitUpdated(habit.clone()));
        Ok(habit)
    }

    pub async fn delete_habit(&self, owner: &AccountId, id: i64) -> Result<(), AppError> {
        let deleted = repositories::habits::delete_habit(&self.pool, id, owner).await?;
        if !deleted {
            return Err(AppError::NotFound(format!("Habit #{id} not found")));
        }
        self.events.send(SystemEvent::HabitDeleted {
            id,
            owner_id: owner.to_string(),
        });
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

        let completed =
            repositories::habits::toggle_log(&self.pool, habit.id, owner, log_date, note).await?;

        self.events.send(SystemEvent::HabitLogToggled {
            habit_id: habit.id,
            owner_id: owner.to_string(),
            log_date: log_date.to_string(),
            completed,
        });

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
