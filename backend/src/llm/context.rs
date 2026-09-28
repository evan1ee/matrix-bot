use crate::db::DbPool;
use crate::events::EventBroadcaster;

/// Runtime context for an LLM turn.
///
/// Encapsulates all dependencies, configuration, and room context needed to
/// execute an LLM conversation turn and its bounded tool loop.
pub struct LlmTurnContext<'a> {
    pub services: &'a crate::application::AppServices,
    pub pool: &'a DbPool,
    pub config: &'a crate::config::AiEnvConfig,
    pub broadcaster: Option<&'a EventBroadcaster>,
    pub owner_id: &'a str,
    pub room_id: &'a str,
    pub stage_reporter: Option<&'a crate::interface::channels::matrix::stage::StageReporter>,
}

impl<'a> LlmTurnContext<'a> {
    pub fn new(
        services: &'a crate::application::AppServices,
        pool: &'a DbPool,
        config: &'a crate::config::AiEnvConfig,
        broadcaster: Option<&'a EventBroadcaster>,
        owner_id: &'a str,
        room_id: &'a str,
        stage_reporter: Option<&'a crate::interface::channels::matrix::stage::StageReporter>,
    ) -> Self {
        Self {
            services,
            pool,
            config,
            broadcaster,
            owner_id,
            room_id,
            stage_reporter,
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
}

pub type LlmTurnResult = AiTurnResult;
