//! Matrix bot and its embedded REST API.
//!
//! Exposed as a library so that auxiliary binaries — notably
//! `generate-openapi` — can reuse the API types instead of the OpenAPI document
//! being produced as a side effect of the test suite.

pub mod accounts;
pub mod api;
pub mod bot;
pub mod config;
pub mod db;
pub mod error;
pub mod events;
pub mod modules;
pub mod utils;
