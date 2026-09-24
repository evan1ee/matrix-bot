//! The LLM transport: any OpenAI-compatible chat-completions API (xAI, OpenAI,
//! Ollama, vLLM, OpenRouter...), reached by base URL alone.
//!
//! Only the wire protocol lives here. Which model a user gets, their saved
//! settings, prompts and the tool loop are `modules::ai`'s business.

pub mod client;
pub mod types;

pub use client::{shared_client, LlmClient, LlmEndpoint, LlmError, StreamedCompletion};
pub use types::*;
