//! The Discord adapter (spec §8.2, plan B).
//!
//! Slash commands (`/todo /expense /portfolio /ask /link /help`, each one
//! string, D6) and DM text are normalized into `IncomingMessage`s for the shared
//! pipeline; replies go back as embeds. serenity stays inside this module.
//!
//! - `commands`: the command table, defer mode, option → `InboundText`.
//! - `inbound`: interaction / DM → `IncomingMessage` (pure).

pub mod commands;
pub mod inbound;

#[cfg(test)]
mod tests {
    //! Discord input through the real pipeline and module registry. No network.

    use std::sync::Arc;

    use super::commands::{inbound_text, COMMANDS};
    use crate::application::AppServices;
    use crate::config::Config;
    use crate::db::{Database, DbPool};
    use crate::domain::accounts::STATUS_ACTIVE;
    use crate::domain::channels::{Channel, ConversationAddress, PlatformIdentity, PlatformUserId};
    use crate::infrastructure::postgres::repositories::accounts::core;
    use crate::infrastructure::postgres::repositories::identities;
    use crate::interface::channels::fake::FakeAdapter;
    use crate::interface::channels::inbound::{Inbound, InboundOutcome, InboundText};
    use crate::interface::channels::matrix::modules::build_modules;
    use crate::interface::channels::message::{IncomingMessage, MessageRef};
    use crate::interface::channels::registry::ChannelRegistry;

    pub(super) const USER: &str = "1100000000000000003";

    /// The registry is kept alive by the returned `Arc`; `Inbound` holds only a `Weak`.
    pub(super) async fn pipeline() -> (Inbound, DbPool, Arc<ChannelRegistry>) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let registry = Arc::new(
            ChannelRegistry::new(vec![Arc::new(FakeAdapter::connected(Channel::Discord))])
                .expect("one adapter"),
        );
        let services = AppServices::new(&pool).with_link_config(
            "https://bot.example".to_string(),
            std::collections::HashMap::new(),
        );
        let inbound = Inbound::new(
            services,
            Database { pool: pool.clone() },
            build_modules(&Config::for_test()).await,
            Arc::downgrade(&registry),
        );
        (inbound, pool, registry)
    }

    pub(super) fn discord_user() -> PlatformIdentity {
        PlatformIdentity::new(
            Channel::Discord,
            PlatformUserId::parse(Channel::Discord, USER).unwrap(),
            None,
        )
    }

    pub(super) async fn link(pool: &DbPool) {
        let account = core::create(pool, "evan", STATUS_ACTIVE).await.unwrap();
        identities::bind(pool, &account, &discord_user())
            .await
            .unwrap();
    }

    /// B1: every command the table dispatches is claimed by the real registry
    /// in the bare form. The account holds no data, so `portfolio` fetches no
    /// prices, and `ai` without a prompt answers its help without the LLM.
    #[tokio::test]
    async fn every_dispatched_command_is_claimed_by_the_registry() {
        let (inbound, pool, _reg) = pipeline().await;
        link(&pool).await;
        for spec in COMMANDS.iter().filter(|c| c.name != "link") {
            let msg = IncomingMessage {
                channel: Channel::Discord,
                conversation: ConversationAddress::Discord(42),
                sender: discord_user(),
                message_id: MessageRef("interaction:1".into()),
                text: String::new(),
                reply_to: None,
                quoted: None,
                is_direct: true,
                is_mentioned: true,
            };
            // `/ask` needs a prompt to pass the adapter; here the registry sees `ai` bare.
            let text = match spec.name {
                "ask" => InboundText::Command {
                    command: "ai".into(),
                    raw_args: String::new(),
                    full_body: "ai".into(),
                },
                name => inbound_text(name, None).unwrap(),
            };
            let outcome = inbound.handle(&msg, text, None).await;
            assert!(
                matches!(outcome, InboundOutcome::Replied(_)),
                "/{} was not claimed",
                spec.name
            );
        }
    }
}
