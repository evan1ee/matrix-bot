//! The Discord adapter (spec §8.2, plan B).
//!
//! Slash commands (`/todo /expense /portfolio /ask /link /help`, each one
//! string, D6) and DM text are normalized into `IncomingMessage`s for the shared
//! pipeline; replies go back as embeds. serenity stays inside this module.
//!
//! - `commands`: the command table, defer mode, option → `InboundText`.
//! - `inbound`: interaction / DM → `IncomingMessage` (pure).
//! - `render`: neutral content → embed posts; where an interaction reply goes (pure).
//! - `outbound`: `send`, the serenity builders, error classification.
//! - `gateway`: serenity's event handler: parse, defer, enqueue (B8).
//! - `worker`: the bounded queue's workers: pipeline call, reply delivery.
//! - `progress`: interaction-editing and typing `ProgressSink`s.
//! - `adapter`: `DiscordAdapter`, its run loop and reconnect backoff (B7).

pub mod adapter;
pub mod commands;
pub mod gateway;
pub mod inbound;
pub mod outbound;
pub mod progress;
pub mod render;
pub mod worker;

#[cfg(test)]
mod tests {
    //! Discord input through the real pipeline and module registry. No network.

    use std::sync::Arc;

    use serenity::all::CommandInteraction;

    use super::commands::{defer_mode, inbound_text, COMMANDS};
    use super::inbound::from_command;
    use super::render::Post;
    use super::worker::plan_command_reply;
    use crate::application::AppServices;
    use crate::config::Config;
    use crate::db::{Database, DbPool};
    use crate::domain::accounts::STATUS_ACTIVE;
    use crate::domain::channels::{Channel, ConversationAddress, PlatformIdentity, PlatformUserId};
    use crate::infrastructure::postgres::repositories::accounts::core;
    use crate::infrastructure::postgres::repositories::{identities, link_tokens};
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

    const FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/channels/discord_command_interaction.json"
    ));

    fn interaction(name: &str, value: Option<&str>, in_guild: bool) -> CommandInteraction {
        let mut v: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        v["data"]["name"] = name.into();
        v["data"]["options"] = match value {
            Some(value) => {
                let option = if name == "ask" { "prompt" } else { "args" };
                serde_json::json!([{ "name": option, "type": 3, "value": value }])
            }
            None => serde_json::json!([]),
        };
        if !in_guild {
            let o = v.as_object_mut().unwrap();
            o.remove("guild_id");
            o.remove("member");
        }
        serde_json::from_value(v).unwrap()
    }

    /// What the deferred response is edited to: normalization → pipeline → plan.
    async fn answer(inbound: &Inbound, i: &CommandInteraction) -> Post {
        let parsed = from_command(i).unwrap();
        let in_guild = i.guild_id.is_some();
        let mode = defer_mode(&i.data.name, in_guild);
        let outcome = inbound.handle(&parsed.message, parsed.text, None).await;
        plan_command_reply(outcome, in_guild, mode)
            .unwrap()
            .original
    }

    fn text(post: &Post) -> &str {
        post.description.as_deref().unwrap_or_default()
    }

    /// Spec invariants 1–2 on Discord.
    #[tokio::test]
    async fn an_unlinked_discord_user_reaches_no_module() {
        let (inbound, _pool, _reg) = pipeline().await;
        let post = answer(&inbound, &interaction("todo", Some("add milk"), true)).await;
        assert!(text(&post).contains("not linked yet"), "{post:?}");
    }

    /// Spec invariant 3 on Discord.
    #[tokio::test]
    async fn link_in_a_guild_is_refused_and_mints_nothing() {
        let (inbound, pool, _reg) = pipeline().await;
        let post = answer(&inbound, &interaction("link", None, true)).await;
        assert!(text(&post).contains("`/link`"), "{post:?}");
        assert!(text(&post).contains("direct message"), "{post:?}");
        assert_eq!(link_tokens::count(&pool).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn link_in_a_dm_returns_the_landing_url() {
        let (inbound, pool, _reg) = pipeline().await;
        let post = answer(&inbound, &interaction("link", None, false)).await;
        assert!(text(&post).contains("bot.example/link?token="), "{post:?}");
        assert_eq!(link_tokens::count(&pool).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn a_linked_user_adds_and_lists_a_todo() {
        let (inbound, pool, _reg) = pipeline().await;
        link(&pool).await;
        let added = answer(&inbound, &interaction("todo", Some("add buy milk"), true)).await;
        assert!(!text(&added).contains("not linked"), "{added:?}");
        let listed = answer(&inbound, &interaction("todo", None, false)).await;
        assert!(text(&listed).contains("buy milk"), "{listed:?}");
    }
}
