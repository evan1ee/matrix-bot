use crate::db::DbPool;
use crate::domain::accounts::AccountId;
use crate::llm::progress::ProgressSink;
use tokio_util::sync::CancellationToken;

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
    /// The prompt embeds text an outsider wrote (a quote from someone other
    /// than the bot, OCR text), so the turn is tainted from the start: see
    /// `llm::guard`.
    pub untrusted_input: bool,
    /// Cancelled when the caller stops the turn. `run_turn` checks it only at
    /// safe points (an LLM call, the top of a round, before the summary),
    /// never while a tool round runs; `None` for channels that cannot stop.
    pub cancel: Option<CancellationToken>,
}

impl<'a> LlmTurnContext<'a> {
    /// Marks the turn as carrying outside content; only a front door that
    /// actually embedded some calls this.
    pub fn with_untrusted_input(mut self) -> Self {
        self.untrusted_input = true;
        self
    }

    /// Lets the caller stop the turn by cancelling `token`.
    pub fn with_cancellation(mut self, token: CancellationToken) -> Self {
        self.cancel = Some(token);
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
            untrusted_input: false,
            cancel: None,
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
