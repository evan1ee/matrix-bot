use std::time::Duration;
use chrono::{DateTime, Utc};

/// Formats a std::time::Duration into human readable string e.g. "10s", "5m 30s", "2h 15m"
pub fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else if secs < 86400 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else {
        format!("{}d {}h", secs / 86400, (secs % 86400) / 3600)
    }
}

/// Parses relative time string like "10s check oven", "5m meeting", "1d pay bill"
pub fn parse_reminder_args(input: &str) -> Option<(Duration, String)> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }

    let mut parts = input.splitn(2, |c: char| c.is_whitespace());
    let time_str = parts.next()?;
    let reminder_msg = parts.next().unwrap_or("Reminder!").trim();

    let (num_str, unit) = time_str.find(|c: char| c.is_alphabetic()).map_or_else(
        || (time_str, "s"),
        |idx| (&time_str[..idx], &time_str[idx..]),
    );

    let num: u64 = num_str.parse().ok()?;
    let secs = match unit.to_lowercase().as_str() {
        "s" | "sec" | "secs" | "second" | "seconds" => num,
        "m" | "min" | "mins" | "minute" | "minutes" => num * 60,
        "h" | "hr" | "hrs" | "hour" | "hours" => num * 3600,
        "d" | "day" | "days" => num * 86400,
        _ => return None,
    };

    if secs == 0 {
        return None;
    }

    Some((Duration::from_secs(secs), reminder_msg.to_string()))
}

/// Returns a human-friendly format for Chrono DateTime<Utc>
pub fn format_datetime(dt: &DateTime<Utc>) -> String {
    dt.format("%Y-%m-%d %H:%M UTC").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::from_secs(45)), "45s");
        assert_eq!(format_duration(Duration::from_secs(125)), "2m 5s");
        assert_eq!(format_duration(Duration::from_secs(3665)), "1h 1m");
        assert_eq!(format_duration(Duration::from_secs(90000)), "1d 1h");
    }

    #[test]
    fn test_parse_reminder_args() {
        let (dur, msg) = parse_reminder_args("10s check oven").unwrap();
        assert_eq!(dur.as_secs(), 10);
        assert_eq!(msg, "check oven");

        let (dur, msg) = parse_reminder_args("5m").unwrap();
        assert_eq!(dur.as_secs(), 300);
        assert_eq!(msg, "Reminder!");
    }
}
