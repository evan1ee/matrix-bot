//! Matrix bot and its embedded REST API.
//!
//! Exposed as a library so that auxiliary binaries — notably
//! `generate-openapi` — can reuse the API types instead of the OpenAPI document
//! being produced as a side effect of the test suite.

// matrix-sdk 0.18's crypto futures nest deeper than the default limit of 128
// when rustc proves the spawned sync task (`app::run`) is `Send`; rustc's own
// suggestion.
#![recursion_limit = "256"]

pub mod app;
pub mod application;
pub mod config;
pub mod db;
pub mod domain;
pub mod error;
pub mod events;
pub mod infrastructure;
pub mod interface;
pub mod jobs;
pub mod llm;
pub mod utils;
