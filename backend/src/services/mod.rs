//! Adapters for capabilities provided outside this process.
//!
//! `modules/` is what the application does; `services/` is how external
//! capabilities are reached. Nothing in here may import from `modules/`.

pub mod llm;
pub mod ocr;
pub mod stt;
pub mod tts;

#[cfg(test)]
pub(crate) mod test_support;
