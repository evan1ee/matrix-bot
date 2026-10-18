//! Monthly spending budgets: pure rules for limits, calendar-month periods in
//! the owner's timezone, threshold crossings and pace projection.
//!
//! All arithmetic is integer minor units; percentage maths widens to `i128` so
//! no input can overflow.

use std::fmt;

use chrono::{DateTime, Datelike, LocalResult, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use super::money::{exponent_for, normalise_currency};
use super::{FinanceError, Transaction, MAX_CATEGORY_LEN};
use crate::domain::accounts::AccountId;

/// What a budget limits: all spending in a currency, or one category's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BudgetScope {
    Overall,
    Category { id: i64, name: String },
}

impl BudgetScope {
    pub fn label(&self) -> &str {
        match self {
            BudgetScope::Overall => "Overall",
            BudgetScope::Category { name, .. } => name,
        }
    }
}

/// A stored monthly limit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    pub id: i64,
    pub owner_id: AccountId,
    pub scope: BudgetScope,
    pub currency: String,
    pub currency_exponent: i64,
    /// Minor units, always positive.
    pub limit_minor: i64,
    pub alerts_enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Validated input for creating a budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewBudget {
    category: Option<String>,
    currency: String,
    currency_exponent: i64,
    limit_minor: i64,
    alerts_enabled: bool,
}

impl NewBudget {
    /// `category: None` (or blank) makes an overall budget.
    pub fn new(
        category: Option<&str>,
        currency: &str,
        limit_minor: i64,
        alerts_enabled: bool,
    ) -> Result<Self, FinanceError> {
        let category = match category.map(str::trim).filter(|c| !c.is_empty()) {
            Some(c) if c.chars().count() > MAX_CATEGORY_LEN => {
                return Err(FinanceError::CategoryNameInvalid {
                    max: MAX_CATEGORY_LEN,
                })
            }
            other => other.map(str::to_owned),
        };
        let currency = normalise_currency(currency)?;
        validate_limit(limit_minor)?;
        let currency_exponent = i64::from(exponent_for(&currency));
        Ok(Self {
            category,
            currency,
            currency_exponent,
            limit_minor,
            alerts_enabled,
        })
    }

    pub fn category(&self) -> Option<&str> {
        self.category.as_deref()
    }
    pub fn currency(&self) -> &str {
        &self.currency
    }
    pub fn currency_exponent(&self) -> i64 {
        self.currency_exponent
    }
    pub fn limit_minor(&self) -> i64 {
        self.limit_minor
    }
    pub fn alerts_enabled(&self) -> bool {
        self.alerts_enabled
    }
}

/// Fields to change on an existing budget.
#[derive(Debug, Default, Clone)]
pub struct BudgetPatch {
    pub limit_minor: Option<i64>,
    pub alerts_enabled: Option<bool>,
}

impl BudgetPatch {
    pub fn validate(&self) -> Result<(), FinanceError> {
        self.limit_minor.map_or(Ok(()), validate_limit)
    }
}

fn validate_limit(limit_minor: i64) -> Result<(), FinanceError> {
    if limit_minor <= 0 {
        return Err(FinanceError::BudgetLimitInvalid);
    }
    Ok(())
}

/// Alert levels, as a percentage of the limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Threshold {
    Warning,
    Exceeded,
}

impl Threshold {
    pub fn pct(self) -> i16 {
        match self {
            Threshold::Warning => 80,
            Threshold::Exceeded => 100,
        }
    }

    pub fn from_pct(pct: i16) -> Option<Self> {
        match pct {
            80 => Some(Threshold::Warning),
            100 => Some(Threshold::Exceeded),
            _ => None,
        }
    }
}

/// Every threshold that `spent_minor` has reached, ascending.
pub fn reached_thresholds(spent_minor: i64, limit_minor: i64) -> Vec<Threshold> {
    [Threshold::Warning, Threshold::Exceeded]
        .into_iter()
        .filter(|t| i128::from(spent_minor) * 100 >= i128::from(limit_minor) * i128::from(t.pct()))
        .collect()
}

/// A calendar month in the owner's timezone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Month {
    year: i32,
    month: u32,
}

impl Month {
    /// The local month that contains the instant `at`.
    pub fn containing(at: DateTime<Utc>, tz: Tz) -> Self {
        let local = at.with_timezone(&tz);
        Self {
            year: local.year(),
            month: local.month(),
        }
    }

    /// Parses `YYYY-MM`.
    pub fn parse(s: &str) -> Result<Self, FinanceError> {
        let invalid = || FinanceError::BudgetMonthInvalid;
        let (y, m) = s.trim().split_once('-').ok_or_else(invalid)?;
        let digits = |t: &str, len: usize| t.len() == len && t.bytes().all(|b| b.is_ascii_digit());
        if !digits(y, 4) || !digits(m, 2) {
            return Err(invalid());
        }
        let (year, month) = (
            y.parse().map_err(|_| invalid())?,
            m.parse().map_err(|_| invalid())?,
        );
        NaiveDate::from_ymd_opt(year, month, 1).ok_or_else(invalid)?;
        Ok(Self { year, month })
    }

    pub fn first_day(&self) -> NaiveDate {
        // Invariant: only constructed from a valid year/month.
        NaiveDate::from_ymd_opt(self.year, self.month, 1).unwrap_or(NaiveDate::MIN)
    }

    fn next_first_day(&self) -> NaiveDate {
        let (year, month) = if self.month == 12 {
            (self.year + 1, 1)
        } else {
            (self.year, self.month + 1)
        };
        NaiveDate::from_ymd_opt(year, month, 1).unwrap_or(NaiveDate::MAX)
    }

    /// First instant of the month in `tz`, as UTC.
    pub fn start_utc(&self, tz: Tz) -> DateTime<Utc> {
        local_midnight_utc(self.first_day(), tz)
    }

    /// First instant of the next month in `tz`, as UTC (exclusive end).
    pub fn end_utc(&self, tz: Tz) -> DateTime<Utc> {
        local_midnight_utc(self.next_first_day(), tz)
    }

    pub fn days(&self) -> u32 {
        (self.next_first_day() - self.first_day()).num_days() as u32
    }
}

/// Local midnight as UTC. If a DST jump skips midnight, the first valid hour
/// after it is used.
fn local_midnight_utc(date: NaiveDate, tz: Tz) -> DateTime<Utc> {
    for hour in 0..4 {
        let Some(naive) = date.and_hms_opt(hour, 0, 0) else {
            continue;
        };
        match tz.from_local_datetime(&naive) {
            LocalResult::Single(t) => return t.with_timezone(&Utc),
            LocalResult::Ambiguous(earliest, _) => return earliest.with_timezone(&Utc),
            LocalResult::None => {}
        }
    }
    date.and_time(chrono::NaiveTime::MIN).and_utc()
}

impl fmt::Display for Month {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}", self.year, self.month)
    }
}

impl From<Month> for String {
    fn from(m: Month) -> Self {
        m.to_string()
    }
}

impl TryFrom<String> for Month {
    type Error = FinanceError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Month::parse(&s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetState {
    Ok,
    Warning,
    Exceeded,
}

/// Spend pace within the current month.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pace {
    /// What an even spend would have reached by the end of today.
    pub expected_minor: i64,
    /// Month-end spend if the current daily rate continues.
    pub projected_minor: i64,
}

/// A budget measured against one month's spending.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetStatus {
    pub budget: Budget,
    pub month: Month,
    pub spent_minor: i64,
    /// Negative once the limit is exceeded.
    pub remaining_minor: i64,
    pub percent_used: i64,
    pub state: BudgetState,
    pub pace: Option<Pace>,
}

/// A transaction together with the budgets it affects, for alert replies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedTransaction {
    pub transaction: Transaction,
    pub budgets: Vec<BudgetStatus>,
}

fn clamp_i64(v: i128) -> i64 {
    i64::try_from(v).unwrap_or(if v < 0 { i64::MIN } else { i64::MAX })
}

/// Measures `budget` against `spent_minor` for `month`. `pace` is present only
/// when `now` falls inside that month.
pub fn status(
    budget: Budget,
    spent_minor: i64,
    month: Month,
    now: DateTime<Utc>,
    tz: Tz,
) -> BudgetStatus {
    let limit = i128::from(budget.limit_minor);
    let spent = i128::from(spent_minor);
    let percent_used = if limit > 0 {
        clamp_i64(spent * 100 / limit)
    } else {
        0
    };
    let state = match reached_thresholds(spent_minor, budget.limit_minor).last() {
        Some(Threshold::Exceeded) => BudgetState::Exceeded,
        Some(Threshold::Warning) => BudgetState::Warning,
        None => BudgetState::Ok,
    };
    let pace = (Month::containing(now, tz) == month).then(|| {
        let elapsed = i128::from(now.with_timezone(&tz).day().max(1));
        let days = i128::from(month.days());
        Pace {
            expected_minor: clamp_i64(limit * elapsed / days),
            projected_minor: clamp_i64(spent * days / elapsed),
        }
    });
    BudgetStatus {
        remaining_minor: budget.limit_minor.saturating_sub(spent_minor),
        budget,
        month,
        spent_minor,
        percent_used,
        state,
        pace,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn budget(limit: i64) -> Budget {
        let at = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        Budget {
            id: 1,
            owner_id: AccountId::generate(),
            scope: BudgetScope::Overall,
            currency: "JPY".into(),
            currency_exponent: 0,
            limit_minor: limit,
            alerts_enabled: true,
            created_at: at,
            updated_at: at,
        }
    }

    #[test]
    fn reached_thresholds_boundaries() {
        assert!(reached_thresholds(7_999, 10_000).is_empty());
        assert_eq!(reached_thresholds(8_000, 10_000), vec![Threshold::Warning]);
        assert_eq!(reached_thresholds(9_999, 10_000), vec![Threshold::Warning]);
        assert_eq!(
            reached_thresholds(10_000, 10_000),
            vec![Threshold::Warning, Threshold::Exceeded]
        );
        assert_eq!(reached_thresholds(15_000, 10_000).len(), 2);
        assert_eq!(reached_thresholds(i64::MAX, i64::MAX).len(), 2);
    }

    #[test]
    fn threshold_pct_round_trips() {
        assert_eq!(Threshold::Warning.pct(), 80);
        assert_eq!(Threshold::Exceeded.pct(), 100);
        assert_eq!(Threshold::from_pct(80), Some(Threshold::Warning));
        assert_eq!(Threshold::from_pct(100), Some(Threshold::Exceeded));
        assert_eq!(Threshold::from_pct(90), None);
    }

    #[test]
    fn month_containing_uses_owner_zone_not_utc() {
        // 2026-10-31T16:30Z is 2026-11-01 00:30 in Asia/Shanghai
        let at = Utc.with_ymd_and_hms(2026, 10, 31, 16, 30, 0).unwrap();
        assert_eq!(
            Month::containing(at, chrono_tz::Asia::Shanghai).to_string(),
            "2026-11"
        );
        assert_eq!(Month::containing(at, chrono_tz::UTC).to_string(), "2026-10");
    }

    #[test]
    fn month_bounds_across_dst_and_year_end() {
        let m = Month::parse("2026-03").unwrap();
        let tz = chrono_tz::Europe::Berlin;
        assert_eq!(
            m.start_utc(tz),
            Utc.with_ymd_and_hms(2026, 2, 28, 23, 0, 0).unwrap()
        );
        assert_eq!(
            m.end_utc(tz),
            Utc.with_ymd_and_hms(2026, 3, 31, 22, 0, 0).unwrap()
        );
        assert_eq!(
            Month::parse("2026-12").unwrap().end_utc(chrono_tz::UTC),
            Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap()
        );
        assert_eq!(Month::parse("2028-02").unwrap().days(), 29);
        assert!(Month::parse("2026-13").is_err() && Month::parse("26-1").is_err());
    }

    #[test]
    fn status_with_no_spend_is_ok() {
        let now = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
        let month = Month::containing(now, chrono_tz::UTC);
        let s = status(budget(10_000), 0, month, now, chrono_tz::UTC);
        assert_eq!(s.state, BudgetState::Ok);
        assert_eq!(s.percent_used, 0);
        assert_eq!(s.remaining_minor, 10_000);
        let pace = s.pace.unwrap();
        assert_eq!((pace.expected_minor, pace.projected_minor), (322, 0));
    }

    #[test]
    fn status_over_limit() {
        let now = Utc.with_ymd_and_hms(2026, 10, 20, 0, 0, 0).unwrap();
        let month = Month::containing(now, chrono_tz::UTC);
        let s = status(budget(10_000), 10_300, month, now, chrono_tz::UTC);
        assert_eq!(s.state, BudgetState::Exceeded);
        assert_eq!(s.percent_used, 103);
        assert_eq!(s.remaining_minor, -300);
    }

    #[test]
    fn status_warning_between_thresholds() {
        let now = Utc.with_ymd_and_hms(2026, 10, 20, 0, 0, 0).unwrap();
        let month = Month::containing(now, chrono_tz::UTC);
        let s = status(budget(10_000), 8_500, month, now, chrono_tz::UTC);
        assert_eq!((s.state, s.percent_used), (BudgetState::Warning, 85));
    }

    #[test]
    fn status_pace_only_for_current_month() {
        let now = Utc.with_ymd_and_hms(2026, 11, 5, 0, 0, 0).unwrap();
        let month = Month::parse("2026-10").unwrap();
        let s = status(budget(10_000), 5_000, month, now, chrono_tz::UTC);
        assert!(s.pace.is_none());
    }

    #[test]
    fn status_pace_on_day_12_of_31() {
        let now = Utc.with_ymd_and_hms(2026, 10, 12, 9, 0, 0).unwrap();
        let month = Month::containing(now, chrono_tz::UTC);
        let s = status(budget(31_000), 15_000, month, now, chrono_tz::UTC);
        let pace = s.pace.unwrap();
        assert_eq!(
            (pace.expected_minor, pace.projected_minor),
            (12_000, 38_750)
        );
    }

    #[test]
    fn new_budget_validation() {
        assert!(matches!(
            NewBudget::new(None, "EUR", 0, true),
            Err(FinanceError::BudgetLimitInvalid)
        ));
        assert!(NewBudget::new(None, "EURO", 100, true).is_err());
        let long = "x".repeat(MAX_CATEGORY_LEN + 1);
        assert!(matches!(
            NewBudget::new(Some(&long), "EUR", 100, true),
            Err(FinanceError::CategoryNameInvalid { .. })
        ));
        let b = NewBudget::new(Some("  "), "kwd", 1_000, true).unwrap();
        assert_eq!(
            (b.category(), b.currency(), b.currency_exponent()),
            (None, "KWD", 3)
        );
        let b = NewBudget::new(Some(" Food "), "eur", 5_000, false).unwrap();
        assert_eq!(
            (b.category(), b.limit_minor(), b.alerts_enabled()),
            (Some("Food"), 5_000, false)
        );
    }

    #[test]
    fn patch_validation() {
        let ok = BudgetPatch {
            limit_minor: Some(1),
            alerts_enabled: None,
        };
        assert!(ok.validate().is_ok());
        let bad = BudgetPatch {
            limit_minor: Some(0),
            alerts_enabled: None,
        };
        assert!(matches!(
            bad.validate(),
            Err(FinanceError::BudgetLimitInvalid)
        ));
    }

    #[test]
    fn month_serialises_as_string() {
        let m = Month::parse("2026-10").unwrap();
        assert_eq!(serde_json::to_string(&m).unwrap(), "\"2026-10\"");
        assert_eq!(serde_json::from_str::<Month>("\"2026-10\"").unwrap(), m);
        assert!(serde_json::from_str::<Month>("\"2026-99\"").is_err());
    }

    #[test]
    fn scope_label_and_serde() {
        assert_eq!(BudgetScope::Overall.label(), "Overall");
        let c = BudgetScope::Category {
            id: 3,
            name: "Food".into(),
        };
        assert_eq!(c.label(), "Food");
        assert_eq!(
            serde_json::to_string(&c).unwrap(),
            r#"{"kind":"category","id":3,"name":"Food"}"#
        );
    }
}
