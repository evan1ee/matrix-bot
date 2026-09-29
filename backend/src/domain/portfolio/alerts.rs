use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::accounts::AccountId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlertCondition {
    Above,
    Below,
}

impl AlertCondition {
    pub fn as_str(&self) -> &'static str {
        match self {
            AlertCondition::Above => "above",
            AlertCondition::Below => "below",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "above" | ">" | "gte" | ">=" => Some(AlertCondition::Above),
            "below" | "<" | "lte" | "<=" => Some(AlertCondition::Below),
            _ => None,
        }
    }

    pub fn symbol_str(&self) -> &'static str {
        match self {
            AlertCondition::Above => "≥",
            AlertCondition::Below => "≤",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceAlert {
    pub id: i64,
    pub owner_id: AccountId,
    pub symbol: String,
    pub target_price_minor: i64,
    pub currency: String,
    pub currency_exponent: i64,
    pub condition: String,
    pub is_triggered: bool,
    pub triggered_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl PriceAlert {
    pub fn parsed_condition(&self) -> AlertCondition {
        AlertCondition::parse(&self.condition).unwrap_or(AlertCondition::Above)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreatePriceAlert {
    pub symbol: String,
    pub target_price: String,
    pub currency: Option<String>,
    pub condition: AlertCondition,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alert_condition_parsing() {
        assert_eq!(AlertCondition::parse("above"), Some(AlertCondition::Above));
        assert_eq!(AlertCondition::parse(">="), Some(AlertCondition::Above));
        assert_eq!(AlertCondition::parse("below"), Some(AlertCondition::Below));
        assert_eq!(AlertCondition::parse("<="), Some(AlertCondition::Below));
        assert_eq!(AlertCondition::parse("invalid"), None);
    }
}
