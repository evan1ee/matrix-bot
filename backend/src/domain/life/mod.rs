pub mod models;

pub use models::{
    normalize_mood, validate_date_str, ActivityEvent, ActivityStats, DayActivityResponse,
    LifeDayLog, LifeHorizonDay, LifeProfile, LifeProfileStats, LifeYearView,
};

#[derive(Debug, thiserror::Error)]
pub enum LifeError {
    #[error("Invalid date: {0}")]
    InvalidDate(String),

    #[error("Invalid mood '{0}'. Valid moods: amazing, good, neutral, low, rough")]
    InvalidMood(String),

    #[error("Expected lifespan must be greater than 0, got {0}")]
    InvalidLifespan(i64),

    #[error("{0}")]
    Validation(String),
}
