use crate::db::DbPool;
use crate::domain::accounts::AccountId;
use crate::llm::progress::ProgressSink;

/// Runtime context for an LLM turn.
///
/// Encapsulates all dependencies, configuration, and room context needed to
/// execute an LLM conversation turn and its bounded tool loop.
pub struct LlmTurnContext<'a> {
    pub services: &'a crate::application::AppServices,
    pub pool: &'a DbPool,
    pub config: &'a crate::config::AiEnvConfig,
    pub owner_id: &'a AccountId,
    pub room_id: &'a str,
    pub progress: Option<&'a dyn ProgressSink>,
    /// The prompt embeds someone else's quoted text, so the turn is tainted
    /// from the start: see `llm::guard`.
    pub quoted_context: bool,
}

impl<'a> LlmTurnContext<'a> {
    /// Marks the turn as carrying quoted content; only the front door that
    /// actually embedded a quote calls this.
    pub fn with_quoted_context(mut self) -> Self {
        self.quoted_context = true;
        self
    }

    pub fn new(
        services: &'a crate::application::AppServices,
        pool: &'a DbPool,
        config: &'a crate::config::AiEnvConfig,
        owner_id: &'a AccountId,
        room_id: &'a str,
        progress: Option<&'a dyn ProgressSink>,
    ) -> Self {
        Self {
            services,
            pool,
            config,
            owner_id,
            room_id,
            progress,
            quoted_context: false,
        }
    }
}

pub type AiTurnContext<'a> = LlmTurnContext<'a>;

/// Result of an LLM turn execution.
#[derive(Debug, Clone)]
pub struct AiTurnResult {
    pub conversation_id: String,
    pub message: String,
    pub model_used: String,
    pub tool_calls_executed: Vec<String>,
    /// (tool name, tool result) pairs, in execution order, for an adapter that links replies to records.
    pub tool_results: Vec<(String, String)>,
}

pub type LlmTurnResult = AiTurnResult;
