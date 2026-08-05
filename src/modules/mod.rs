use async_trait::async_trait;
use matrix_sdk::Client;
use std::sync::Arc;
use tracing::info;

use crate::db::Database;

pub mod core;
pub mod reminder;
pub mod todo;

#[derive(Clone, Debug)]
pub struct CommandContext {
    pub room_id: String,
    pub sender_mxid: String,
    pub sender_name: String,
    pub command: String,
    pub raw_args: String,
    pub full_body: String,
}

#[derive(Clone)]
pub struct ModuleContext {
    pub client: Client,
    pub db: Database,
    pub broadcaster: Option<crate::events::EventBroadcaster>,
}

#[derive(Clone, Debug)]
pub struct CommandHelp {
    #[allow(dead_code)]
    pub name: String,
    pub usage: String,
    pub description: String,
}

#[async_trait]
pub trait BotModule: Send + Sync {
    fn name(&self) -> &str;
    fn commands(&self) -> Vec<CommandHelp>;
    async fn handle_command(&self, ctx: &CommandContext, db: &Database) -> Option<String>;
    async fn spawn_background_workers(&self, _ctx: &ModuleContext) {}
}

#[derive(Default)]
pub struct ModuleRegistry {
    modules: Vec<Arc<dyn BotModule>>,
}

impl ModuleRegistry {
    pub fn new() -> Self {
        Self { modules: Vec::new() }
    }

    pub fn register<M: BotModule + 'static>(&mut self, module: M) {
        info!("Registering bot module: {}", module.name());
        self.modules.push(Arc::new(module));
    }

    pub async fn dispatch(&self, ctx: &CommandContext, db: &Database) -> Option<String> {
        for module in &self.modules {
            if let Some(reply) = module.handle_command(ctx, db).await {
                return Some(reply);
            }
        }
        None
    }

    pub async fn spawn_all_background_workers(&self, ctx: &ModuleContext) {
        for module in &self.modules {
            module.spawn_background_workers(ctx).await;
        }
    }

    pub fn get_all_help(&self) -> Vec<(String, Vec<CommandHelp>)> {
        self.modules
            .iter()
            .map(|m| (m.name().to_string(), m.commands()))
            .collect()
    }
}
