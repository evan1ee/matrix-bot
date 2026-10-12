//! Standard 5-field cron expression validation and next-occurrence evaluation.
//!
//! Format: `minute hour day_of_month month day_of_week` (evaluated in UTC).
//! Supports `*`, `*/step`, `a-b`, `a-b/step`, comma-separated lists, and
//! 3-letter month (`JAN`–`DEC`) and weekday (`SUN`–`SAT`, `0` or `7` = Sunday)
//! names.

use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};

use super::AgentError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSchedule {
    minutes: u64, // bits 0..=59
    hours: u32,   // bits 0..=23
    days: u32,    // bits 1..=31
    months: u16,  // bits 1..=12
    weekdays: u8, // bits 0..=6 (0 = Sunday)
    dom_wildcard: bool,
    dow_wildcard: bool,
}

impl CronSchedule {
    pub fn parse(expr: &str) -> Result<Self, AgentError> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(AgentError::InvalidCronExpression(
                "cron expression must have exactly 5 fields (minute hour day month weekday)"
                    .to_string(),
            ));
        }

        let minutes = parse_field(fields[0], 0, 59, &[])?;
        let hours = parse_field(fields[1], 0, 23, &[])? as u32;
        let days = parse_field(fields[2], 1, 31, &[])? as u32;
        let months = parse_field(
            fields[3],
            1,
            12,
            &[
                ("JAN", 1),
                ("FEB", 2),
                ("MAR", 3),
                ("APR", 4),
                ("MAY", 5),
                ("JUN", 6),
                ("JUL", 7),
                ("AUG", 8),
                ("SEP", 9),
                ("OCT", 10),
                ("NOV", 11),
                ("DEC", 12),
            ],
        )? as u16;
        let raw_weekdays = parse_field(
            fields[4],
            0,
            7,
            &[
                ("SUN", 0),
                ("MON", 1),
                ("TUE", 2),
                ("WED", 3),
                ("THU", 4),
                ("FRI", 5),
                ("SAT", 6),
            ],
        )? as u8;
        let weekdays = (raw_weekdays & 0x7f) | ((raw_weekdays >> 7) & 1);

        let schedule = Self {
            minutes,
            hours,
            days,
            months,
            weekdays,
            dom_wildcard: fields[2] == "*",
            dow_wildcard: fields[4] == "*",
        };

        // Ensure the schedule has at least one reachable instant (rejects e.g. Feb 31).
        let probe = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        schedule.next_after(probe)?;

        Ok(schedule)
    }

    /// Computes the next UTC instant strictly after `after` matching this cron schedule.
    pub fn next_after(&self, after: DateTime<Utc>) -> Result<DateTime<Utc>, AgentError> {
        let start = (after + Duration::minutes(1))
            .with_second(0)
            .and_then(|dt| dt.with_nanosecond(0))
            .ok_or_else(|| {
                AgentError::InvalidCronExpression("invalid reference timestamp".to_string())
            })?;

        let limit = start + Duration::days(366 * 5);
        let mut dt = start;

        while dt <= limit {
            if (self.months & (1u16 << dt.month())) == 0 {
                let (ny, nm) = if dt.month() == 12 {
                    (dt.year() + 1, 1)
                } else {
                    (dt.year(), dt.month() + 1)
                };
                dt = Utc
                    .with_ymd_and_hms(ny, nm, 1, 0, 0, 0)
                    .single()
                    .ok_or_else(|| {
                        AgentError::InvalidCronExpression("date overflow".to_string())
                    })?;
                continue;
            }

            if !self.matches_day(dt) {
                dt = (dt + Duration::days(1))
                    .with_hour(0)
                    .and_then(|d| d.with_minute(0))
                    .unwrap();
                continue;
            }

            if (self.hours & (1u32 << dt.hour())) == 0 {
                dt = (dt + Duration::hours(1)).with_minute(0).unwrap();
                continue;
            }

            if (self.minutes & (1u64 << dt.minute())) == 0 {
                dt += Duration::minutes(1);
                continue;
            }

            return Ok(dt);
        }

        Err(AgentError::InvalidCronExpression(
            "cron expression has no reachable occurrence within 5 years".to_string(),
        ))
    }

    fn matches_day(&self, dt: DateTime<Utc>) -> bool {
        let dom_match = (self.days & (1u32 << dt.day())) != 0;
        let dow = dt.weekday().num_days_from_sunday();
        let dow_match = (self.weekdays & (1u8 << dow)) != 0;

        match (self.dom_wildcard, self.dow_wildcard) {
            (true, true) => true,
            (true, false) => dow_match,
            (false, true) => dom_match,
            // Standard Vixie cron: if both DOM and DOW are restricted, either matches.
            (false, false) => dom_match || dow_match,
        }
    }
}

fn parse_field(raw: &str, min: u32, max: u32, names: &[(&str, u32)]) -> Result<u64, AgentError> {
    let mut mask = 0u64;
    for part in raw.split(',') {
        if part.is_empty() {
            return Err(AgentError::InvalidCronExpression(format!(
                "empty token in cron field '{raw}'"
            )));
        }
        let (range_part, step) = match part.split_once('/') {
            Some((r, s)) => {
                let step: u32 = s.parse().map_err(|_| {
                    AgentError::InvalidCronExpression(format!("invalid step '{s}' in '{raw}'"))
                })?;
                if step == 0 || step > max {
                    return Err(AgentError::InvalidCronExpression(format!(
                        "step '{step}' out of range in '{raw}'"
                    )));
                }
                (r, step)
            }
            None => (part, 1),
        };

        let (start, end) = if range_part == "*" {
            (min, max)
        } else if let Some((a, b)) = range_part.split_once('-') {
            let lo = parse_atom(a, min, max, names)?;
            let hi = parse_atom(b, min, max, names)?;
            if lo > hi {
                return Err(AgentError::InvalidCronExpression(format!(
                    "invalid range '{range_part}' in '{raw}'"
                )));
            }
            (lo, hi)
        } else {
            let v = parse_atom(range_part, min, max, names)?;
            if part.contains('/') {
                (v, max)
            } else {
                (v, v)
            }
        };

        let mut v = start;
        while v <= end {
            mask |= 1u64 << v;
            v += step;
        }
    }
    if mask == 0 {
        return Err(AgentError::InvalidCronExpression(format!(
            "cron field '{raw}' matched no values"
        )));
    }
    Ok(mask)
}

fn parse_atom(token: &str, min: u32, max: u32, names: &[(&str, u32)]) -> Result<u32, AgentError> {
    let upper = token.to_ascii_uppercase();
    if let Some((_, val)) = names.iter().find(|(n, _)| *n == upper) {
        return Ok(*val);
    }
    let v: u32 = token
        .parse()
        .map_err(|_| AgentError::InvalidCronExpression(format!("invalid cron value '{token}'")))?;
    if v < min || v > max {
        return Err(AgentError::InvalidCronExpression(format!(
            "cron value '{v}' out of range {min}..={max}"
        )));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_and_named_cron_expressions() {
        let base = Utc.with_ymd_and_hms(2026, 10, 11, 8, 30, 0).unwrap(); // Sunday
        let monday_9am = CronSchedule::parse("0 9 * * 1").unwrap();
        assert_eq!(
            monday_9am.next_after(base).unwrap(),
            Utc.with_ymd_and_hms(2026, 10, 12, 9, 0, 0).unwrap()
        );

        let sunday_9pm = CronSchedule::parse("0 21 * * SUN").unwrap();
        assert_eq!(
            sunday_9pm.next_after(base).unwrap(),
            Utc.with_ymd_and_hms(2026, 10, 11, 21, 0, 0).unwrap()
        );

        let every_15 = CronSchedule::parse("*/15 * * * *").unwrap();
        assert_eq!(
            every_15.next_after(base).unwrap(),
            Utc.with_ymd_and_hms(2026, 10, 11, 8, 45, 0).unwrap()
        );
    }

    #[test]
    fn rejects_invalid_and_impossible_cron_expressions() {
        for bad in [
            "",
            "invalid",
            "* * * *",
            "60 * * * *",
            "0 24 * * *",
            "0 0 31 2 *",
        ] {
            assert!(
                CronSchedule::parse(bad).is_err(),
                "expected '{bad}' to be rejected"
            );
        }
    }
}
