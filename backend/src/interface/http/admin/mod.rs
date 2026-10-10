//! Administrative REST API endpoints for configuration, secrets, and auditing.

pub mod audit;
pub mod secrets;
pub mod settings;
pub mod status;

#[cfg(test)]
mod tests;
