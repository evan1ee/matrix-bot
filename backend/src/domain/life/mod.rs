pub mod models;

/// What a caller supplies when recording a day: the mood is required, every
/// other field is optional and left as it was when omitted.
pub struct DayLogInput<'a> {
    pub mood: &'a str,
    pub weather: Option<&'a str>,
    pub activities: Option<&'a str>,
    pub note: Option<&'a str>,
    pub time_allocation: Option<&'a str>,
}

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
