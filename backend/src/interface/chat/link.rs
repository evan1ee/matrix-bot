//! The `link` command, answered before the sender is resolved.
//!
//! Ordering is the whole reason this is not an ordinary `BotModule`: every
//! other command runs after the sender has been resolved to an account, and
//! resolving first would provision an empty account for the very identity
//! being linked (Matrix) or refuse it (every other channel). So the pipeline
//! calls this on the raw identity, for every channel, and it never provisions.
//!
//! The token is minted by `LinkService` and only ever appears in the reply
//! (a direct conversation); it is never logged here.

use crate::application::{AppError, AppServices};
use crate::domain::channels::Channel;
use crate::interface::channels::message::{IncomingMessage, Reply};
use crate::interface::chat::error_reply;
use crate::utils::html::html_escape;

/// How a user on `channel` types a command: Matrix keeps its `!`, every newer
/// channel uses a slash.
fn command_prefix(channel: Channel) -> char {
    match channel {
        Channel::Matrix => '!',
        _ => '/',
    }
}

/// Handles `link` (arguments, such as the retired web-minted code, are ignored).
pub async fn handle_link(services: &AppServices, msg: &IncomingMessage) -> Reply {
    let label = html_escape(msg.channel.label());
    match services
        .link
        .start(&msg.sender, &msg.conversation, msg.is_direct)
        .await
    {
        Ok(started) => {
            let url = html_escape(&started.url);
            // The token is a credential: a private reply, so a channel that
            // can also post in public never does with this one.
            Reply::private(format!(
                "Open this link within 10 minutes to connect this {label} account to your \
                 workspace:<br/><a href=\"{url}\">{url}</a>"
            ))
        }
        Err(AppError::Validation(_)) => Reply::html(format!(
            "That would be visible to everyone here. Send <code>{}link</code> to me in a \
             direct message.",
            command_prefix(msg.channel)
        )),
        Err(err) => error_reply(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::channels::{Channel, ConversationAddress, PlatformIdentity, PlatformUserId};
    use crate::infrastructure::postgres::repositories::link_tokens;
    use crate::interface::channels::message::{IncomingMessage, MessageRef};

    fn message(channel: Channel, raw: &str, is_direct: bool) -> IncomingMessage {
        let sender =
            PlatformIdentity::new(channel, PlatformUserId::parse(channel, raw).unwrap(), None);
        let conversation = match channel {
            Channel::Matrix => ConversationAddress::Matrix("!dm:x".into()),
            _ => ConversationAddress::Telegram(42),
        };
        IncomingMessage {
            channel,
            conversation,
            sender,
            message_id: MessageRef("m".into()),
            text: String::new(),
            reply_to: None,
            quoted: None,
            is_direct,
            is_mentioned: false,
        }
    }

    async fn services() -> (AppServices, sqlx::PgPool) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let services = AppServices::new(&pool).with_link_config(
            "https://bot.example".to_string(),
            std::collections::HashMap::new(),
        );
        (services, pool)
    }

    #[tokio::test]
    async fn a_shared_conversation_is_told_to_use_a_direct_message_and_mints_nothing() {
        let (services, pool) = services().await;
        let reply = handle_link(&services, &message(Channel::Telegram, "42", false)).await;
        assert!(
            reply.html_body().contains("direct message"),
            "{}",
            reply.html_body()
        );
        assert!(reply.html_body().contains("<code>/link</code>"));
        assert_eq!(link_tokens::count(&pool).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn a_direct_conversation_gets_the_landing_url() {
        let (services, pool) = services().await;
        let reply = handle_link(&services, &message(Channel::Telegram, "42", true)).await;
        let html = reply.html_body();
        assert!(html.contains("https://bot.example/link?token="), "{html}");
        assert!(html.contains("within 10 minutes"));
        assert!(html.contains("this Telegram account"));
        assert!(html.contains("<a href=\""));
        assert_eq!(link_tokens::count(&pool).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn matrix_users_are_pointed_at_the_bang_command() {
        let (services, _pool) = services().await;
        let reply = handle_link(&services, &message(Channel::Matrix, "@evan:mbm.one", false)).await;
        assert!(reply.html_body().contains("<code>!link</code>"));
        let reply = handle_link(&services, &message(Channel::Matrix, "@evan:mbm.one", true)).await;
        assert!(reply.html_body().contains("this Matrix account"));
    }
}
