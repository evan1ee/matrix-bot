//! PostgreSQL repositories, one file per feature: plain `async fn`s over
//! `impl sqlx::PgExecutor`, compile-checked `query!` SQL, private row structs
//! mapped into domain types before they leave the file.

pub mod rooms;
