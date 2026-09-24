//! Matrix bot and its embedded REST API.
//!
//! Exposed as a library so that auxiliary binaries — notably
//! `generate-openapi` — can reuse the API types instead of the OpenAPI document
//! being produced as a side effect of the test suite.

pub mod accounts;
pub mod application;
pub mod config;
pub mod db;
pub mod domain;
pub mod error;
pub mod events;
pub mod infrastructure;
pub mod interface;
pub mod modules;
pub mod utils;
