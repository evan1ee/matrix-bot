//! Pure iCalendar (RFC 5545) generation for the read-only calendar feed.

pub mod feed;
pub mod ics;

pub use feed::{build_feed, should_emit_reminder, CalendarInput, MAX_FEED_ITEMS};
