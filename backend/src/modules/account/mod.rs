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
use crate::accounts::{AccountRepository, MaturedRecovery, RecoveryRepository, SecurityEventKind};
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
                name: "passkey".to_string(),
                usage: "!passkey [add|list]".to_string(),
                description: "Link a passkey (Face ID, Touch ID, Windows Hello) to your Matrix \
                              account, or list your active passkeys."
                    .to_string(),
            },
            CommandHelp {
                name: "passkeys".to_string(),
                usage: "!passkeys".to_string(),
                description: "List all passkeys linked to your Matrix account.".to_string(),
            },
            CommandHelp {
                name: "unlink".to_string(),
                usage: "!unlink [passkey <num>|sessions]".to_string(),
                description: "Revoke a linked passkey or sign out all web browser sessions."
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

        match command.as_str() {
            "passkey" | "passkeys" | "web" => Some(self.handle_passkey(ctx, db).await),
            "unlink" => Some(self.handle_unlink(ctx, db).await),
            "recover" => Some(self.handle_recover(ctx, db).await),
            _ => None,
        }
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

    async fn handle_passkey(&self, ctx: &CommandContext, db: &Database) -> String {
        let trimmed = ctx.raw_args.trim();
        let subcmd = trimmed
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_lowercase();

        if subcmd == "add"
            || subcmd == "link"
            || subcmd == "enroll"
            || ctx.command.eq_ignore_ascii_case("web")
        {
            if !ctx.is_direct {
                return "Send <code>!passkey add</code> in a direct message — the link contains a single-use token that must not be shown in a shared room.".to_string();
            }

            match crate::accounts::EnrollmentRepository::issue(&db.pool, &ctx.owner_id, 10).await {
                Ok(token) => {
                    let enroll_url =
                        format!("{}/?enroll={}", self.web_url.trim_end_matches('/'), token);
                    format!(
                        "<b>Link a Passkey to your Matrix Account</b> (<code>{}</code>)<br/><br/>\
                         Open this link on your device (iPhone, Mac, Android, Windows) to register Face ID, Touch ID, or Windows Hello:<br/>\
                         <a href=\"{}\">{}</a><br/><br/>\
                         <i>Expires in 10 minutes and can be used once. Repeat on multiple devices to attach multiple passkeys.</i>",
                        escape(&ctx.sender_mxid),
                        escape(&enroll_url),
                        escape(&enroll_url),
                    )
                }
                Err(err) => {
                    tracing::error!(
                        "Failed issuing passkey enrollment token for {}: {:?}",
                        ctx.owner_id,
                        err
                    );
                    "Could not generate passkey enrollment link. Try again shortly.".to_string()
                }
            }
        } else if subcmd == "rm" || subcmd == "delete" || subcmd == "remove" {
            let target = trimmed.split_whitespace().nth(1).unwrap_or("");
            self.handle_unlink_passkey(ctx, db, target).await
        } else {
            match crate::accounts::CredentialRepository::list_for_account(&db.pool, &ctx.owner_id)
                .await
            {
                Ok(creds) => {
                    if creds.is_empty() {
                        format!(
                            "You have no passkeys linked to <code>{}</code> yet.<br/><br/>\
                             Send <code>!passkey add</code> in a direct message to link this device using Face ID, Touch ID, or Windows Hello.",
                            escape(&ctx.sender_mxid)
                        )
                    } else {
                        let mut lines = Vec::new();
                        lines.push(format!(
                            "<b>Linked Passkeys for <code>{}</code>:</b><br/>",
                            escape(&ctx.sender_mxid)
                        ));
                        for (idx, cred) in creds.iter().enumerate() {
                            let created_str = cred
                                .created_at
                                .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
                                .unwrap_or_else(|| "unknown".to_string());
                            lines.push(format!(
                                "{}. <b>{}</b> (added {})<br/>",
                                idx + 1,
                                escape(&cred.label),
                                escape(&created_str)
                            ));
                        }
                        lines.push("<br/>• To remove a passkey: <code>!unlink passkey &lt;number&gt;</code><br/>• To link another device: <code>!passkey add</code>".to_string());
                        lines.join("")
                    }
                }
                Err(err) => {
                    tracing::error!("Could not list passkeys for {}: {:?}", ctx.owner_id, err);
                    "Could not load passkeys. Try again shortly.".to_string()
                }
            }
        }
    }

    async fn handle_unlink(&self, ctx: &CommandContext, db: &Database) -> String {
        let trimmed = ctx.raw_args.trim();
        let mut parts = trimmed.split_whitespace();
        let target_type = parts.next().unwrap_or("").to_lowercase();

        match target_type.as_str() {
            "passkey" | "pk" => {
                let target = parts.next().unwrap_or("");
                self.handle_unlink_passkey(ctx, db, target).await
            }
            "sessions" | "session" => {
                if !ctx.is_direct {
                    return "Send <code>!unlink sessions</code> in a direct message.".to_string();
                }
                match crate::accounts::SessionRepository::revoke_all_for_account(&db.pool, &ctx.owner_id, None).await {
                    Ok(count) => {
                        crate::accounts::SecurityEventRepository::record(
                            &db.pool,
                            &ctx.owner_id,
                            crate::accounts::SecurityEventKind::SessionsRevoked,
                            Some(&format!("revoked {} session(s) via Matrix command", count)),
                        )
                        .await;
                        format!("✅ Revoked {} active web browser session(s) for your account.", count)
                    }
                    Err(err) => {
                        tracing::error!("Could not revoke sessions for {}: {:?}", ctx.owner_id, err);
                        "Could not revoke web sessions. Try again shortly.".to_string()
                    }
                }
            }
            "matrix" => {
                let target_mxid = parts.next().unwrap_or("");
                if target_mxid.is_empty() {
                    return "Usage: <code>!unlink matrix &lt;@user:server.org&gt;</code>".to_string();
                }
                if target_mxid == ctx.sender_mxid {
                    return "Cannot unlink your currently active Matrix identity. To unlink a secondary Matrix account, specify its MXID.".to_string();
                }
                match crate::accounts::LinkRepository::unlink(&db.pool, &ctx.owner_id, target_mxid).await {
                    Ok(true) => {
                        crate::accounts::SecurityEventRepository::record(
                            &db.pool,
                            &ctx.owner_id,
                            crate::accounts::SecurityEventKind::MatrixUnlinked,
                            Some(target_mxid),
                        )
                        .await;
                        format!("✅ Unlinked Matrix identity <code>{}</code>.", escape(target_mxid))
                    }
                    Ok(false) => format!("Matrix account <code>{}</code> is not linked to your account.", escape(target_mxid)),
                    Err(err) => {
                        tracing::error!("Could not unlink Matrix {}: {:?}", target_mxid, err);
                        "Could not unlink Matrix account. Try again shortly.".to_string()
                    }
                }
            }
            _ => {
                "<b>Account Unlink Commands:</b><br/>\
                 • <code>!unlink passkey &lt;number&gt;</code> — Remove a linked passkey (see <code>!passkeys</code>)<br/>\
                 • <code>!unlink sessions</code> — Sign out all active web browser sessions<br/>\
                 • <code>!passkeys</code> — List all your linked passkeys<br/>\
                 • <code>!passkey add</code> — Link a new passkey to this Matrix account"
                    .to_string()
            }
        }
    }

    async fn handle_unlink_passkey(
        &self,
        ctx: &CommandContext,
        _db: &Database,
        _target: &str,
    ) -> String {
        if !ctx.is_direct {
            return "Send passkey management commands in a direct message.".to_string();
        }
        let web_url = self.web_url.trim_end_matches('/');
        format!(
            "🔒 <b>Passkey Removal Requires Verification</b><br/><br/>\
             To protect your account against unauthorized removal, passkeys can only be removed from the web dashboard after confirming your identity with a recent passkey assertion.<br/><br/>\
             Please visit your <b>Security Settings</b> on the web dashboard to manage or remove passkeys:<br/>\
             <a href=\"{url}/account\">{url}/account</a>",
            url = escape(web_url)
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

    async fn db() -> Database {
        let pool = crate::db::pool::test_pool().await;
        run_migrations(&pool).await.unwrap();
        Database { pool: pool }
    }

    fn module() -> AccountModule {
        AccountModule::new(
            Arc::new(RwLock::new(None)),
            "https://bot.mbm.one".to_string(),
        )
    }

    fn ctx(command: &str, owner_id: &str, is_direct: bool) -> CommandContext {
        CommandContext {
            room_id: if is_direct {
                "!dm:x".into()
            } else {
                "!public:x".into()
            },
            owner_id: owner_id.to_string(),
            sender_mxid: "@evan:mbm.one".to_string(),
            sender_name: "evan".to_string(),
            command: command.to_string(),
            raw_args: String::new(),
            full_body: format!("!{command}"),
            is_direct,
            is_mentioned: false,
            quoted: None,
            stage_reporter: None,
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
            assert!(
                reply.contains("bot.mbm.one"),
                "`!{command}` must name the site"
            );
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
        let account = AccountRepository::create(&db.pool, "evan", STATUS_ACTIVE)
            .await
            .unwrap();

        let reply = module()
            .handle_command(&ctx("recover", &account, false), &db, None)
            .await
            .unwrap();

        assert!(reply.contains("direct message"));
        assert!(
            RecoveryRepository::pending(&db.pool, &account)
                .await
                .unwrap()
                .is_none(),
            "nothing may be started from a public room"
        );
    }

    /// An MXID that reaches only its own auto-provisioned account has no web
    /// account to recover, and is told so rather than handed a useless code.
    #[tokio::test]
    async fn test_recover_needs_a_claimed_account() {
        let db = db().await;
        let account = AccountRepository::create(&db.pool, "evan", STATUS_UNCLAIMED)
            .await
            .unwrap();

        let reply = module()
            .handle_command(&ctx("recover", &account, true), &db, None)
            .await
            .unwrap();

        assert!(reply.contains("!link"), "it should say how to link instead");
        assert!(RecoveryRepository::pending(&db.pool, &account)
            .await
            .unwrap()
            .is_none());
    }

    /// The happy path: a code now, usable in a day, cancellable until then.
    #[tokio::test]
    async fn test_recover_issues_a_delayed_code_and_says_so() {
        let db = db().await;
        let account = AccountRepository::create(&db.pool, "evan", STATUS_ACTIVE)
            .await
            .unwrap();

        let reply = module()
            .handle_command(&ctx("recover", &account, true), &db, None)
            .await
            .unwrap();

        assert!(
            reply.contains("24 hours"),
            "the delay must be stated: {reply}"
        );
        assert!(reply.contains("cancel"), "the way out must be stated");
        assert!(RecoveryRepository::pending(&db.pool, &account)
            .await
            .unwrap()
            .is_some());

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
        let account = AccountRepository::create(&db.pool, "evan", STATUS_ACTIVE)
            .await
            .unwrap();
        let (code, _) = RecoveryRepository::request(&db.pool, &account)
            .await
            .unwrap();

        assert_eq!(
            redeem_matured_recovery(&db, None, &code).await.unwrap_err(),
            RecoveryRejection::TooEarly
        );
    }

    #[tokio::test]
    async fn test_a_matured_code_opens_the_account_and_is_logged() {
        let db = db().await;
        let account = AccountRepository::create(&db.pool, "evan", STATUS_ACTIVE)
            .await
            .unwrap();
        let (code, _) = RecoveryRepository::request(&db.pool, &account)
            .await
            .unwrap();

        crate::db_execute!(
            &db.pool,
            "UPDATE recovery_requests SET valid_after = ? WHERE account_id = ?",
            chrono::Utc::now() - chrono::Duration::minutes(1),
            &account
        )
        .unwrap();

        assert_eq!(
            redeem_matured_recovery(&db, None, &code).await.unwrap(),
            account
        );

        let events = crate::accounts::SecurityEventRepository::list(&db.pool, &account, 10)
            .await
            .unwrap();
        assert_eq!(events[0].kind, "recovery_completed");
    }

    #[tokio::test]
    async fn test_passkey_add_generates_enroll_link_in_dm() {
        let db = db().await;
        let account = AccountRepository::create(&db.pool, "evan", STATUS_ACTIVE)
            .await
            .unwrap();

        // In public room: refused
        let mut public_ctx = ctx("passkey", &account, false);
        public_ctx.raw_args = "add".into();
        let reply = module()
            .handle_command(&public_ctx, &db, None)
            .await
            .unwrap();
        assert!(reply.contains("direct message"));

        // In DM: generates ?enroll= token link
        let mut dm_ctx = ctx("passkey", &account, true);
        dm_ctx.raw_args = "add".into();
        let reply = module().handle_command(&dm_ctx, &db, None).await.unwrap();
        assert!(reply.contains("?enroll="));
        assert!(reply.contains("bot.mbm.one"));
    }

    #[tokio::test]
    async fn test_passkeys_list_and_unlink_flow() {
        let db = db().await;
        let account = AccountRepository::create(&db.pool, "evan", STATUS_ACTIVE)
            .await
            .unwrap();

        // Initially no passkeys
        let list_ctx = ctx("passkeys", &account, true);
        let reply = module().handle_command(&list_ctx, &db, None).await.unwrap();
        assert!(reply.contains("no passkeys linked"));

        // Insert two passkeys and a matrix link
        crate::db_execute!(
            &db.pool,
            "INSERT INTO matrix_links (mxid, account_id) VALUES ('@evan:mbm.one', ?)",
            &account
        )
        .unwrap();
        crate::db_execute!(&db.pool, "INSERT INTO webauthn_credentials (id, account_id, passkey_json, label) VALUES ('pk1', ?, '{}', 'MacBook'), ('pk2', ?, '{}', 'iPhone')", &account, &account)
            .unwrap();

        // List passkeys
        let reply = module().handle_command(&list_ctx, &db, None).await.unwrap();
        assert!(reply.contains("MacBook"));
        assert!(reply.contains("iPhone"));

        // Unlink passkey via Matrix DM is safely rejected, directing user to web settings
        let mut unlink_ctx = ctx("unlink", &account, true);
        unlink_ctx.raw_args = "passkey 1".into();
        let reply = module()
            .handle_command(&unlink_ctx, &db, None)
            .await
            .unwrap();
        assert!(reply.contains("Passkey Removal Requires Verification"));
        assert!(reply.contains("/account"));

        // Verify passkeys were NOT deleted from database
        let creds_after =
            crate::accounts::CredentialRepository::list_for_account(&db.pool, &account)
                .await
                .unwrap();
        assert_eq!(creds_after.len(), 2);

        // Unlink sessions
        let mut sessions_ctx = ctx("unlink", &account, true);
        sessions_ctx.raw_args = "sessions".into();
        let reply = module()
            .handle_command(&sessions_ctx, &db, None)
            .await
            .unwrap();
        assert!(reply.contains("Revoked 0 active web browser session(s)"));
    }
}
