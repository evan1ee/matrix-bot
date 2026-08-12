//! The Matrix half of account management.
//!
//! Two commands, and a wall.
//!
//! `!link` and `!recover` live here because they can only be done from Matrix —
//! each proves control of a Matrix identity, which is exactly what the web UI
//! cannot do for you.
//!
//! Everything else is refused. The Matrix path has no way to prompt for a
//! passkey, so it has no route to elevated privilege at all — that is what makes
//! "sensitive actions require recent passkey verification" a real guarantee
//! rather than a decoration. A command that hits the wall replies with a pointer
//! to the web app rather than failing silently, because a user who typed
//! `!export` deserves to know where to go.

use async_trait::async_trait;
use matrix_sdk::Client;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::accounts::recovery::RECOVERY_DELAY_HOURS;
use crate::accounts::{
    AccountRepository, MaturedRecovery, RecoveryRepository, SecurityEventKind,
};
use crate::bot::notify::notify_account;
use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::modules::{BotModule, CommandContext, CommandHelp};
use crate::utils::html::html_escape as escape;

/// Commands that exist on the web only, and what to say about each.
///
/// Table-driven so the refusal and the help text cannot drift apart, and so a
/// new sensitive command has one obvious place to be added.
const WEB_ONLY: &[(&str, &str)] = &[
    ("export", "Exporting financial data"),
    ("unlink", "Unlinking a Matrix account"),
    ("passkey", "Managing passkeys"),
    ("passkeys", "Managing passkeys"),
    ("sessions", "Managing signed-in devices"),
    ("recoverycodes", "Generating recovery codes"),
];

pub struct AccountModule {
    /// This generation's Matrix client, for sending notifications.
    ///
    /// Held behind the same shared handle the API uses, so a reconnect swaps it
    /// out underneath rather than leaving this module holding a dead client.
    client: Arc<RwLock<Option<Client>>>,
    /// Where to tell people to go. Shown in every refusal.
    web_url: String,
}

impl AccountModule {
    pub fn new(client: Arc<RwLock<Option<Client>>>, web_url: String) -> Self {
        Self { client, web_url }
    }
}

#[async_trait]
impl BotModule for AccountModule {
    fn name(&self) -> &str {
        "Account"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "link".to_string(),
                usage: "!link <code>".to_string(),
                description: "Attach this Matrix account to your web account. \
                              Get a code from the web app, then send it here in a \
                              direct message."
                    .to_string(),
            },
            CommandHelp {
                name: "recover".to_string(),
                usage: "!recover".to_string(),
                description: format!(
                    "Start account recovery if you have lost your passkeys. \
                     Takes {RECOVERY_DELAY_HOURS} hours and can be cancelled from \
                     any signed-in device."
                ),
            },
        ]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        db: &Database,
        _broadcaster: Option<&EventBroadcaster>,
    ) -> Option<String> {
        let command = ctx.command.to_lowercase();

        if let Some((_, what)) = WEB_ONLY.iter().find(|(name, _)| *name == command) {
            return Some(format!(
                "<b>{}</b> needs a passkey, so it is only available in the web app.<br/>\
                 Open <code>{}</code> to do it there.",
                escape(what),
                escape(&self.web_url)
            ));
        }

        if command == "recover" {
            return Some(self.handle_recover(ctx, db).await);
        }

        None
    }
}

impl AccountModule {
    /// Starts recovery for the account this MXID reaches.
    ///
    /// Refused outside a direct message: the code is in the reply, and a reply
    /// in a shared room would hand it to everyone present.
    async fn handle_recover(&self, ctx: &CommandContext, db: &Database) -> String {
        if !ctx.is_direct {
            return "Send <code>!recover</code> in a direct message — the reply contains \
                    a code that must not be shown to a room."
                .to_string();
        }

        // The sender's account is whatever this MXID reaches. An MXID linked to
        // nothing has only the empty account the resolver just made it, and
        // recovering that is meaningless.
        let account = match AccountRepository::find(&db.pool, &ctx.owner_id).await {
            Ok(Some(account)) => account,
            _ => return "Could not read your account. Try again shortly.".to_string(),
        };

        if account.is_unclaimed() {
            return format!(
                "This Matrix account is not linked to a web account, so there is nothing \
                 to recover.<br/>Register at <code>{}</code> and link it with \
                 <code>!link</code>.",
                escape(&self.web_url)
            );
        }

        let (code, valid_after) = match RecoveryRepository::request(&db.pool, &ctx.owner_id).await {
            Ok(result) => result,
            Err(err) => {
                tracing::error!("Could not start recovery for {}: {:?}", ctx.owner_id, err);
                return "Could not start recovery. Try again shortly.".to_string();
            }
        };

        // Announced to *every* linked identity, not just this one. If someone
        // else started this, the real owner has to hear about it — on a channel
        // the attacker may not control.
        let client = self.client.read().await.clone();
        notify_account(
            client.as_ref(),
            db,
            &ctx.owner_id,
            SecurityEventKind::RecoveryRequested,
            Some(&format!("requested from {}", ctx.sender_mxid)),
        )
        .await;

        format!(
            "Recovery started.<br/><br/>Your code is <code>{}</code>.<br/><br/>\
             It will not work until <b>{}</b> — {RECOVERY_DELAY_HOURS} hours from now. \
             Until then, anyone still signed in can cancel this.<br/><br/>\
             When it becomes active you can use it to register a new passkey. \
             It will not let you unlink accounts, remove passkeys, or export \
             financial data for a further 72 hours.",
            escape(&code),
            escape(&valid_after.format("%Y-%m-%d %H:%M UTC").to_string()),
        )
    }
}

/// Redeems a matured recovery code presented in the web UI.
///
/// Lives here rather than in `api/` because the whole flow — request, delay,
/// warning — is one story, and splitting its two halves across modules would
/// make the fence harder to see.
pub async fn redeem_matured_recovery(
    db: &Database,
    client: Option<&Client>,
    code: &str,
) -> Result<String, RecoveryRejection> {
    match RecoveryRepository::redeem_request(&db.pool, code).await {
        Ok(MaturedRecovery::Ready { account_id }) => {
            notify_account(
                client,
                db,
                &account_id,
                SecurityEventKind::RecoveryCompleted,
                None,
            )
            .await;
            Ok(account_id)
        }
        Ok(MaturedRecovery::TooEarly) => Err(RecoveryRejection::TooEarly),
        Ok(MaturedRecovery::Unavailable) => Err(RecoveryRejection::Unavailable),
        Err(err) => {
            tracing::error!("Could not redeem a recovery request: {:?}", err);
            Err(RecoveryRejection::Unavailable)
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RecoveryRejection {
    TooEarly,
    Unavailable,
}

#[cfg(test)]
mod tests {
    use super::*;
use crate::accounts::{STATUS_ACTIVE, STATUS_UNCLAIMED};
    use crate::db::schema::run_migrations;
    use sqlx::SqlitePool;

    async fn db() -> Database {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        Database { pool }
    }

    fn module() -> AccountModule {
        AccountModule::new(Arc::new(RwLock::new(None)), "https://bot.mbm.one".to_string())
    }

    fn ctx(command: &str, owner_id: &str, is_direct: bool) -> CommandContext {
        CommandContext {
            room_id: if is_direct { "!dm:x".into() } else { "!public:x".into() },
            owner_id: owner_id.to_string(),
            sender_mxid: "@evan:mbm.one".to_string(),
            sender_name: "evan".to_string(),
            command: command.to_string(),
            raw_args: String::new(),
            full_body: format!("!{command}"),
            is_direct,
        }
    }

    /// The ceiling. Every one of these needs a passkey, and Matrix cannot
    /// prompt for one — so none of them may work here, whatever the sender's
    /// account can do on the web.
    #[tokio::test]
    async fn test_sensitive_commands_are_refused_with_a_pointer() {
        let db = db().await;
        let module = module();

        for (command, _) in WEB_ONLY {
            let reply = module
                .handle_command(&ctx(command, "acct_evan", true), &db, None)
                .await
                .unwrap_or_else(|| panic!("`!{command}` must be answered, not ignored"));

            assert!(
                reply.contains("web app"),
                "`!{command}` must explain where to go: {reply}"
            );
            assert!(reply.contains("bot.mbm.one"), "`!{command}` must name the site");
        }
    }

    /// An ordinary command must fall through to the module that owns it.
    #[tokio::test]
    async fn test_unrelated_commands_are_not_claimed() {
        let db = db().await;
        assert!(module()
            .handle_command(&ctx("todo", "acct_evan", true), &db, None)
            .await
            .is_none());
    }

    /// The reply carries the recovery code, so a shared room must not get one.
    #[tokio::test]
    async fn test_recover_is_refused_outside_a_direct_message() {
        let db = db().await;
        let account = AccountRepository::create(&db.pool, "evan", STATUS_ACTIVE).await.unwrap();

        let reply = module()
            .handle_command(&ctx("recover", &account, false), &db, None)
            .await
            .unwrap();

        assert!(reply.contains("direct message"));
        assert!(
            RecoveryRepository::pending(&db.pool, &account).await.unwrap().is_none(),
            "nothing may be started from a public room"
        );
    }

    /// An MXID that reaches only its own auto-provisioned account has no web
    /// account to recover, and is told so rather than handed a useless code.
    #[tokio::test]
    async fn test_recover_needs_a_claimed_account() {
        let db = db().await;
        let account = AccountRepository::create(&db.pool, "evan", STATUS_UNCLAIMED).await.unwrap();

        let reply = module()
            .handle_command(&ctx("recover", &account, true), &db, None)
            .await
            .unwrap();

        assert!(reply.contains("!link"), "it should say how to link instead");
        assert!(RecoveryRepository::pending(&db.pool, &account).await.unwrap().is_none());
    }

    /// The happy path: a code now, usable in a day, cancellable until then.
    #[tokio::test]
    async fn test_recover_issues_a_delayed_code_and_says_so() {
        let db = db().await;
        let account = AccountRepository::create(&db.pool, "evan", STATUS_ACTIVE).await.unwrap();

        let reply = module()
            .handle_command(&ctx("recover", &account, true), &db, None)
            .await
            .unwrap();

        assert!(reply.contains("24 hours"), "the delay must be stated: {reply}");
        assert!(reply.contains("cancel"), "the way out must be stated");
        assert!(RecoveryRepository::pending(&db.pool, &account).await.unwrap().is_some());

        // And it is logged, so the web UI shows it even with no DM delivered.
        let events = crate::accounts::SecurityEventRepository::list(&db.pool, &account, 10)
            .await
            .unwrap();
        assert_eq!(events[0].kind, "recovery_requested");
    }

    /// A code presented before it matures must be refused, not honoured early.
    #[tokio::test]
    async fn test_a_premature_code_is_refused_in_the_web_ui_too() {
        let db = db().await;
        let account = AccountRepository::create(&db.pool, "evan", STATUS_ACTIVE).await.unwrap();
        let (code, _) = RecoveryRepository::request(&db.pool, &account).await.unwrap();

        assert_eq!(
            redeem_matured_recovery(&db, None, &code).await.unwrap_err(),
            RecoveryRejection::TooEarly
        );
    }

    #[tokio::test]
    async fn test_a_matured_code_opens_the_account_and_is_logged() {
        let db = db().await;
        let account = AccountRepository::create(&db.pool, "evan", STATUS_ACTIVE).await.unwrap();
        let (code, _) = RecoveryRepository::request(&db.pool, &account).await.unwrap();

        sqlx::query("UPDATE recovery_requests SET valid_after = ? WHERE account_id = ?")
            .bind(chrono::Utc::now() - chrono::Duration::minutes(1))
            .bind(&account)
            .execute(&db.pool)
            .await
            .unwrap();

        assert_eq!(redeem_matured_recovery(&db, None, &code).await.unwrap(), account);

        let events = crate::accounts::SecurityEventRepository::list(&db.pool, &account, 10)
            .await
            .unwrap();
        assert_eq!(events[0].kind, "recovery_completed");
    }
}
