//! Adapters for capabilities provided outside this process.
//!
//! `interface/` and `application/` define what the application does; `infrastructure/` is how external
//! capabilities are reached. Nothing in here may import from `interface/` or `application/`.

pub mod embedding;
pub mod llm;
pub mod ocr;
pub mod stt;
pub mod testers;
pub mod tts;

#[cfg(test)]
pub(crate) mod test_support;
