//! The Telegram adapter (spec §8.3, plan C).
//!
//! Bot API updates (messages, `/commands`, mentions, callback-query button
//! presses) are normalized into `IncomingMessage`s for the shared pipeline.
//! `teloxide-core` stays inside this module.
//!
//! - `commands`: `/name[@bot] args` parsing, the advertised command list.
//! - `inbound`: `Update` → `IncomingMessage` (pure).
//! - `render`: `Content` → planned Telegram messages and keyboard (pure).
//! - `outbound`: `send`, the only code that calls the Bot API for a reply.
//! - `ingress`: the bounded update queue and its de-duplication (T11).
//! - `worker`: the queue's consumers: parse, pipeline call, reply delivery.
//! - `progress`: the typing / status-message `ProgressSink` (T15).
//! - `polling`: the `getUpdates` loop.
//! - `adapter`: `TelegramAdapter`, start-up, webhook registration, backoff.

pub mod adapter;
pub mod commands;
pub mod inbound;
pub mod ingress;
pub mod outbound;
pub mod polling;
pub mod progress;
pub mod render;
pub mod worker;

#[cfg(test)]
mod tests {
    //! Telegram updates through the real pipeline and module registry. No network.

    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use super::inbound::UpdateKind;
    use super::ingress::RecentUpdates;
    use super::render::PRIVATE_IN_GROUP;
    use super::worker::{dispatch, plan_reply, prepare, Prepared};
    use crate::application::AppServices;
    use crate::config::Config;
    use crate::db::{Database, DbPool};
    use crate::domain::accounts::STATUS_ACTIVE;
    use crate::domain::channels::{Channel, PlatformIdentity, PlatformUserId};
    use crate::infrastructure::postgres::repositories::accounts::core;
    use crate::infrastructure::postgres::repositories::{identities, link_tokens, todos};
    use crate::interface::channels::fake::FakeAdapter;
    use crate::interface::channels::inbound::{Inbound, InboundOutcome, InboundText};
    use crate::interface::channels::matrix::modules::build_modules;
    use crate::interface::channels::registry::ChannelRegistry;
    use crate::interface::channels::telegram::render::Part;

    const PRIVATE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/channels/telegram_private_text.json"
    ));
    const GROUP: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/channels/telegram_group_command.json"
    ));
    const CALLBACK: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/channels/telegram_callback_query.json"
    ));
    /// The private-text fixture's sender.
    const EVAN: &str = "4100000003";
    /// The callback fixture's presser.
    const DANA: &str = "4100000004";

    async fn pipeline() -> (Inbound, DbPool, Arc<ChannelRegistry>) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let registry = Arc::new(
            ChannelRegistry::new(vec![Arc::new(FakeAdapter::connected(Channel::Telegram))])
                .expect("one adapter"),
        );
        let services = AppServices::new(&pool)
            .with_link_config("https://bot.example".to_string(), HashMap::new());
        let inbound = Inbound::new(
            services,
            Database { pool: pool.clone() },
            build_modules(&Config::for_test()).await,
            Arc::downgrade(&registry),
        );
        (inbound, pool, registry)
    }

    fn identity(id: &str) -> PlatformIdentity {
        PlatformIdentity::new(
            Channel::Telegram,
            PlatformUserId::parse(Channel::Telegram, id).unwrap(),
            None,
        )
    }

    async fn link(pool: &DbPool, id: &str, name: &str) -> crate::domain::accounts::AccountId {
        let account = core::create(pool, name, STATUS_ACTIVE).await.unwrap();
        identities::bind(pool, &account, &identity(id))
            .await
            .unwrap();
        account
    }

    fn body(fixture: &str, edit: impl FnOnce(&mut serde_json::Value)) -> String {
        let mut v: serde_json::Value = serde_json::from_str(fixture).unwrap();
        edit(&mut v);
        v.to_string()
    }

    /// update bytes → parse → dispatch → plan; the reply's text parts joined.
    async fn answer(inbound: &Inbound, raw: &str) -> (String, UpdateKind) {
        let recent = Mutex::new(RecentUpdates::new(16));
        let Prepared::Dispatch(n) = prepare(raw.as_bytes(), &recent, "MyBot") else {
            panic!("the update was not dispatched");
        };
        let in_group = !n.message.is_direct;
        let outcome = dispatch(inbound, &n.message, n.text, None).await;
        let plan = plan_reply(outcome, in_group).unwrap().expect("a reply");
        let text = plan
            .parts
            .iter()
            .filter_map(|p| match p {
                Part::Text(t) => Some(t.as_str()),
                Part::File(_) => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        (text, n.kind)
    }

    fn private(text: &str) -> String {
        body(PRIVATE, |v| v["message"]["text"] = text.into())
    }

    /// Spec invariants 1-2 on Telegram.
    #[tokio::test]
    async fn an_unlinked_telegram_user_reaches_no_module() {
        let (inbound, _pool, _reg) = pipeline().await;
        let (text, _) = answer(&inbound, &private("/todo add milk")).await;
        assert!(text.contains("not linked yet"), "{text}");
    }

    /// Spec invariant 3 on Telegram: refused in a group, nothing minted.
    #[tokio::test]
    async fn link_in_a_group_is_refused_and_mints_nothing() {
        let (inbound, pool, _reg) = pipeline().await;
        let raw = body(GROUP, |v| {
            v["message"]["text"] = "/link".into();
            v["message"].as_object_mut().unwrap().remove("entities");
        });
        let (text, _) = answer(&inbound, &raw).await;
        assert!(text.contains("/link"), "{text}");
        assert_eq!(link_tokens::count(&pool).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn a_linked_user_adds_and_lists_a_todo() {
        let (inbound, pool, _reg) = pipeline().await;
        link(&pool, EVAN, "evan").await;
        let (added, _) = answer(&inbound, &private("/todo add buy milk")).await;
        assert!(!added.contains("not linked"), "{added}");
        let (listed, _) = answer(&inbound, &private("/todo")).await;
        assert!(listed.contains("buy milk"), "{listed}");
    }

    /// T4: `/start` is `/help` for a linked user, once.
    #[tokio::test]
    async fn a_linked_start_answers_with_help() {
        let (inbound, pool, _reg) = pipeline().await;
        link(&pool, EVAN, "evan").await;
        let raw = Mutex::new(RecentUpdates::new(4));
        let Prepared::Dispatch(n) = prepare(private("/start").as_bytes(), &raw, "MyBot") else {
            panic!("start was not dispatched");
        };
        // The pipeline itself ignores `start` for a linked user; `dispatch` does the re-dispatch.
        let start = InboundText::Command {
            command: "start".into(),
            raw_args: String::new(),
            full_body: "start".into(),
        };
        let direct = inbound.handle(&n.message, start, None).await;
        assert!(matches!(direct, InboundOutcome::Ignored));
        let outcome = dispatch(&inbound, &n.message, n.text, None).await;
        let InboundOutcome::Replied(reply) = outcome else {
            panic!("start was not answered");
        };
        assert!(
            !reply.content.plain.contains("not linked"),
            "{:?}",
            reply.content
        );
    }

    /// T4: an unlinked `/start` keeps the pipeline's own answer (no second dispatch).
    #[tokio::test]
    async fn an_unlinked_start_gets_the_link_hint() {
        let (inbound, _pool, _reg) = pipeline().await;
        let (text, _) = answer(&inbound, &private("/start")).await;
        assert!(text.contains("not linked yet"), "{text}");
    }

    /// Spec invariant 9: a button is dispatched as the presser.
    #[tokio::test]
    async fn pressing_someone_elses_done_button_changes_nothing() {
        let (inbound, pool, _reg) = pipeline().await;
        let evan = link(&pool, EVAN, "evan").await;
        link(&pool, DANA, "dana").await;
        answer(&inbound, &private("/todo add buy milk")).await;
        let item = todos::list_all_user(&pool, &evan, None, 10, 0)
            .await
            .unwrap()
            .remove(0);
        let press = body(CALLBACK, |v| {
            v["callback_query"]["data"] = format!("todo done {}", item.id).into();
        });
        let (text, kind) = answer(&inbound, &press).await;
        assert!(text.contains("not found"), "{text}");
        assert!(matches!(kind, UpdateKind::Callback { .. }));
        assert!(
            !todos::find(&pool, item.id)
                .await
                .unwrap()
                .unwrap()
                .completed
        );
    }

    #[tokio::test]
    async fn pressing_your_own_done_button_completes_it() {
        let (inbound, pool, _reg) = pipeline().await;
        let evan = link(&pool, EVAN, "evan").await;
        answer(&inbound, &private("/todo add buy milk")).await;
        let item = todos::list_all_user(&pool, &evan, None, 10, 0)
            .await
            .unwrap()
            .remove(0);
        let press = body(CALLBACK, |v| {
            v["callback_query"]["from"]["id"] = 4_100_000_003_i64.into();
            v["callback_query"]["data"] = format!("todo done {}", item.id).into();
        });
        let (text, _) = answer(&inbound, &press).await;
        assert!(text.contains("completed"), "{text}");
        assert!(
            todos::find(&pool, item.id)
                .await
                .unwrap()
                .unwrap()
                .completed
        );
    }

    /// Spec invariant 10: a private reply is never posted in a group.
    #[test]
    fn a_private_reply_planned_for_a_group_is_only_the_notice() {
        use crate::interface::channels::message::Reply;
        let plan = plan_reply(InboundOutcome::Replied(Reply::private("secret")), true)
            .unwrap()
            .unwrap();
        let texts: Vec<_> = plan
            .parts
            .iter()
            .filter_map(|p| match p {
                Part::Text(t) => Some(t.as_str()),
                Part::File(_) => None,
            })
            .collect();
        assert_eq!(texts, [PRIVATE_IN_GROUP]);
    }
}
