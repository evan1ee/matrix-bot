//! Compile-time proof of the assumption the dual-dialect data layer rests on:
//! a single `#[derive(sqlx::FromRow)]` model satisfies `FromRow` for *both*
//! `SqliteRow` and `PgRow`, including its `chrono` columns.
//!
//! If this stops compiling, the design in `db/dialect.rs` is unsound and every
//! model would need a second, driver-specific mapping. That is worth a
//! compile error rather than a discovery halfway through the rollout.

#![cfg(test)]

use chrono::{DateTime, Utc};
use sqlx::postgres::PgRow;
use sqlx::sqlite::SqliteRow;
use sqlx::FromRow;

/// Shaped like `modules::todo::models::Todo` — the awkward parts are the
/// `Option<DateTime<Utc>>` and the `bool`, which the two drivers store
/// differently (SQLite has neither type natively).
#[derive(FromRow)]
#[allow(dead_code)]
struct Probe {
    id: i64,
    text: String,
    completed: bool,
    due_at: Option<DateTime<Utc>>,
    created_at: Option<DateTime<Utc>>,
}

fn assert_readable_from<'r, R, T>()
where
    R: sqlx::Row,
    T: FromRow<'r, R>,
{
}

#[test]
fn test_one_model_derive_serves_both_drivers() {
    assert_readable_from::<SqliteRow, Probe>();
    assert_readable_from::<PgRow, Probe>();
}
