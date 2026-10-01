//! The Matrix half of account management.
//!
//! Two commands, a first passkey, and a wall.
//!
//! `!link` and `!recover` live here because they can only be done from Matrix —
//! each proves control of a Matrix identity, which is exactly what the web UI
//! cannot do for you. `!passkey add` enrolls a passkey only for an account that
//! has none, where the Matrix identity is already the only way in; listing
//! passkeys is read-only.
//!
//! Everything else is refused. The Matrix path has no way to prompt for a
//! passkey, so it has no route to elevated privilege at all — that is what makes
//! "sensitive actions require recent passkey verification" a real guarantee
//! rather than a decoration. A command that hits the wall replies with a pointer
//! to the web app rather than failing silently, because a user who typed
//! `!export` deserves to know where to go.

use async_trait::async_trait;

use crate::interface::channels::message::Reply;
use std::sync::Weak;

use crate::application::AppServices;
use crate::db::Database;
use crate::domain::accounts::{
    AccountId, MaturedRecovery, SecurityEventKind, RECOVERY_DELAY_HOURS,
};
use crate::interface::channels::matrix::notify::announce_account;
use crate::interface::channels::registry::ChannelRegistry;
use crate::interface::chat::{BotModule, CommandContext, CommandHelp};
use crate::utils::html::html_escape as escape;

/// Commands that exist on the web only, and what to say about each.
///
/// Table-driven so the refusal and the help text cannot drift apart, and so a
/// new sensitive command has one obvious place to be added.
const WEB_ONLY: &[(&str, &str)] = &[
    ("export", "Exporting financial data"),
    ("sessions", "Managing signed-in devices"),
    ("recoverycodes", "Generating recovery codes"),
    (
        "unlink",
        "Unlinking passkeys, Matrix accounts and signed-in devices",
    ),
];

pub struct AccountModule {
    /// Where to tell people to go. Shown in every refusal.
    web_url: String,
}

impl AccountModule {
    pub fn new(web_url: String) -> Self {
        Self { web_url }
    }
}

impl AccountModule {
    /// The command's HTML answer; `handle_command` wraps it in a [`Reply`].
    async fn respond(&self, ctx: &CommandContext, services: &AppServices) -> Option<String> {
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
            "passkey" | "passkeys" | "web" => Some(self.handle_passkey(ctx, services).await),
            "recover" => Some(self.handle_recover(ctx, services).await),
            _ => None,
        }
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
                description: "Add a first passkey (Face ID, Touch ID, Windows Hello) to an \
                              account that has none, or list your active passkeys."
                    .to_string(),
            },
            CommandHelp {
                name: "passkeys".to_string(),
                usage: "!passkeys".to_string(),
                description: "List all passkeys linked to your Matrix account.".to_string(),
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
        services: &AppServices,
        _db: &Database,
    ) -> Option<Reply> {
        self.respond(ctx, services).await.map(Reply::private)
    }
}

impl AccountModule {
    /// Starts recovery for the account this MXID reaches.
    ///
    /// Refused outside a direct message: the code is in the reply, and a reply
    /// in a shared room would hand it to everyone present.
    async fn handle_recover(&self, ctx: &CommandContext, services: &AppServices) -> String {
        if !ctx.is_direct {
            return "Send <code>!recover</code> in a direct message — the reply contains \
                    a code that must not be shown to a room."
                .to_string();
        }

        // The sender's account is whatever this MXID reaches. An MXID linked to
        // nothing has only the empty account the resolver just made it, and
        // recovering that is meaningless.
        let account = match services.accounts.find(&ctx.owner_id).await {
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

        let detail = format!("requested from {}", ctx.sender.user_id().as_str());
        let (code, valid_after) = match services
            .accounts
            .request_recovery(&ctx.owner_id, Some(&detail))
            .await
        {
            Ok(result) => result,
            Err(err) => {
                tracing::error!("Could not start recovery for {}: {:?}", ctx.owner_id, err);
                return "Could not start recovery. Try again shortly.".to_string();
            }
        };

        // Announced to *every* linked identity, not just this one. If someone
        // else started this, the real owner has to hear about it — on a channel
        // the attacker may not control.
        announce_account(
            ctx.outbound.as_ref(),
            &services.identity,
            &ctx.owner_id,
            SecurityEventKind::RecoveryRequested,
            Some(&detail),
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

    async fn handle_passkey(&self, ctx: &CommandContext, services: &AppServices) -> String {
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

            // First enrollment only. Once a passkey exists, a link from here
            // would let the Matrix account alone add another and sign in
            // fully verified — exactly what the `!recover` delay exists to stop.
            match services.accounts.count_passkeys(&ctx.owner_id).await {
                Ok(0) => {}
                Ok(_) => {
                    return format!(
                        "This account already has a passkey, so another can only be added \
                         from a signed-in device at <code>{}</code>.<br/>\
                         Lost every passkey? Send <code>!recover</code> instead.",
                        escape(&self.web_url)
                    );
                }
                Err(err) => {
                    tracing::error!("Could not count passkeys for {}: {:?}", ctx.owner_id, err);
                    return "Could not generate passkey enrollment link. Try again shortly."
                        .to_string();
                }
            }

            match services
                .accounts
                .issue_enrollment_token(&ctx.owner_id, 10)
                .await
            {
                Ok(token) => {
                    let enroll_url =
                        format!("{}/?enroll={}", self.web_url.trim_end_matches('/'), token);
                    format!(
                        "<b>Link a Passkey to your Matrix Account</b> (<code>{}</code>)<br/><br/>\
                         Open this link on your device (iPhone, Mac, Android, Windows) to register Face ID, Touch ID, or Windows Hello:<br/>\
                         <a href=\"{}\">{}</a><br/><br/>\
                         <i>Expires in 10 minutes and can be used once. Add more passkeys later from the web app.</i>",
                        escape(ctx.sender_mxid().unwrap_or_default()),
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
            self.handle_remove_passkey(ctx)
        } else {
            match services.accounts.list_passkeys(&ctx.owner_id).await {
                Ok(creds) => {
                    if creds.is_empty() {
                        format!(
                            "You have no passkeys linked to <code>{}</code> yet.<br/><br/>\
                             Send <code>!passkey add</code> in a direct message to link this device using Face ID, Touch ID, or Windows Hello.",
                            escape(ctx.sender.user_id().as_str())
                        )
                    } else {
                        let mut lines = Vec::new();
                        lines.push(format!(
                            "<b>Linked Passkeys for <code>{}</code>:</b><br/>",
                            escape(ctx.sender.user_id().as_str())
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
                        lines.push(format!(
                            "<br/>Add or remove passkeys from a signed-in device at <code>{}</code>.",
                            escape(&self.web_url)
                        ));
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

    fn handle_remove_passkey(&self, ctx: &CommandContext) -> String {
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
pub async fn redeem_matured_recovery(
    services: &AppServices,
    outbound: Option<&Weak<ChannelRegistry>>,
    code: &str,
) -> Result<AccountId, RecoveryRejection> {
    match services.accounts.redeem_recovery_request(code).await {
        Ok(MaturedRecovery::Ready { account_id }) => {
            announce_account(
                outbound,
                &services.identity,
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
    use crate::domain::accounts::{STATUS_ACTIVE, STATUS_UNCLAIMED};
    use crate::domain::channels::Channel;
    use crate::infrastructure::postgres::repositories::accounts as repo;

    async fn setup() -> (Database, AppServices) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let services = AppServices::new(&pool);
        (Database { pool }, services)
    }

    fn module() -> AccountModule {
        AccountModule::new("https://bot.mbm.one".to_string())
    }

    fn ctx(command: &str, owner_id: &AccountId, is_direct: bool) -> CommandContext {
        CommandContext {
            conversation: crate::domain::channels::ConversationAddress::Matrix(if is_direct {
                "!dm:x".into()
            } else {
                "!public:x".into()
            }),
            owner_id: *owner_id,
            sender: crate::interface::chat::test_support::sender("@evan:mbm.one", "evan"),
            command: command.to_string(),
            raw_args: String::new(),
            full_body: format!("!{command}"),
            is_direct,
            is_mentioned: false,
            quoted: None,
            progress: None,
            outbound: None,
        }
    }

    #[tokio::test]
    async fn test_sensitive_commands_are_refused_with_a_pointer() {
        let (db, services) = setup().await;
        let module = module();
        let acct = AccountId::from_seed("acct_evan");

        for (command, _) in WEB_ONLY {
            let reply = module
                .handle_command(&ctx(command, &acct, true), &services, &db)
                .await
                .unwrap_or_else(|| panic!("`!{command}` must be answered, not ignored"))
                .html_body()
                .to_owned();

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

    #[tokio::test]
    async fn test_unrelated_commands_are_not_claimed() {
        let (db, services) = setup().await;
        let acct = AccountId::from_seed("acct_evan");
        assert!(module()
            .handle_command(&ctx("todo", &acct, true), &services, &db)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn test_recover_is_refused_outside_a_direct_message() {
        let (db, services) = setup().await;
        let account = services
            .accounts
            .create("evan", STATUS_ACTIVE)
            .await
            .unwrap();

        let reply = module()
            .handle_command(&ctx("recover", &account, false), &services, &db)
            .await
            .unwrap()
            .html_body()
            .to_owned();

        assert!(reply.contains("direct message"));
        assert!(
            services
                .accounts
                .pending_recovery(&account)
                .await
                .unwrap()
                .is_none(),
            "nothing may be started from a public room"
        );
    }

    #[tokio::test]
    async fn test_recover_needs_a_claimed_account() {
        let (db, services) = setup().await;
        let account = services
            .accounts
            .create("evan", STATUS_UNCLAIMED)
            .await
            .unwrap();

        let reply = module()
            .handle_command(&ctx("recover", &account, true), &services, &db)
            .await
            .unwrap()
            .html_body()
            .to_owned();

        assert!(reply.contains("!link"), "it should say how to link instead");
        assert!(services
            .accounts
            .pending_recovery(&account)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn test_recover_issues_a_delayed_code_and_says_so() {
        let (db, services) = setup().await;
        let account = services
            .accounts
            .create("evan", STATUS_ACTIVE)
            .await
            .unwrap();

        let reply = module()
            .handle_command(&ctx("recover", &account, true), &services, &db)
            .await
            .unwrap()
            .html_body()
            .to_owned();

        assert!(
            reply.contains("24 hours"),
            "the delay must be stated: {reply}"
        );
        assert!(reply.contains("cancel"), "the way out must be stated");
        assert!(services
            .accounts
            .pending_recovery(&account)
            .await
            .unwrap()
            .is_some());

        let events = services
            .accounts
            .list_security_events(&account, 10)
            .await
            .unwrap();
        assert_eq!(events[0].kind, "recovery_requested");
    }

    #[tokio::test]
    async fn test_a_premature_code_is_refused_in_the_web_ui_too() {
        let (_db, services) = setup().await;
        let account = services
            .accounts
            .create("evan", STATUS_ACTIVE)
            .await
            .unwrap();
        let (code, _) = services
            .accounts
            .request_recovery(&account, None)
            .await
            .unwrap();

        assert_eq!(
            redeem_matured_recovery(&services, None, &code)
                .await
                .unwrap_err(),
            RecoveryRejection::TooEarly
        );
    }

    #[tokio::test]
    async fn test_a_matured_code_opens_the_account_and_is_logged() {
        let (db, services) = setup().await;
        let account = services
            .accounts
            .create("evan", STATUS_ACTIVE)
            .await
            .unwrap();
        let (code, _) = services
            .accounts
            .request_recovery(&account, None)
            .await
            .unwrap();

        repo::recovery::set_valid_after(
            &db.pool,
            &account,
            chrono::Utc::now() - chrono::Duration::minutes(1),
        )
        .await
        .unwrap();

        assert_eq!(
            redeem_matured_recovery(&services, None, &code)
                .await
                .unwrap(),
            account
        );

        let events = services
            .accounts
            .list_security_events(&account, 10)
            .await
            .unwrap();
        assert_eq!(events[0].kind, "recovery_completed");
    }

    async fn audit_rows(
        services: &AppServices,
        account: &AccountId,
    ) -> Vec<(String, Option<String>)> {
        let mut rows: Vec<_> = services
            .accounts
            .list_security_events(account, 50)
            .await
            .unwrap()
            .into_iter()
            .map(|e| (e.kind, e.detail))
            .collect();
        rows.sort();
        rows
    }

    /// `!recover` leaves exactly one `recovery_requested` row, carrying who asked.
    #[tokio::test]
    async fn test_recover_records_one_request_row_with_the_requester() {
        let (db, services) = setup().await;
        let account = services
            .accounts
            .create("evan", STATUS_ACTIVE)
            .await
            .unwrap();

        module()
            .handle_command(&ctx("recover", &account, true), &services, &db)
            .await
            .unwrap();

        assert_eq!(
            audit_rows(&services, &account).await,
            vec![(
                "recovery_requested".to_string(),
                Some("requested from @evan:mbm.one".to_string())
            )]
        );
    }

    /// Redeeming a matured code is one recovery: one `recovery_completed` row.
    #[tokio::test]
    async fn test_a_matured_code_records_recovery_completed_exactly_once() {
        let (db, services) = setup().await;
        let account = services
            .accounts
            .create("evan", STATUS_ACTIVE)
            .await
            .unwrap();
        let (code, _) = services
            .accounts
            .request_recovery(&account, None)
            .await
            .unwrap();
        repo::recovery::set_valid_after(
            &db.pool,
            &account,
            chrono::Utc::now() - chrono::Duration::minutes(1),
        )
        .await
        .unwrap();

        redeem_matured_recovery(&services, None, &code)
            .await
            .unwrap();

        assert_eq!(
            audit_rows(&services, &account).await,
            vec![
                ("recovery_completed".to_string(), None),
                ("recovery_requested".to_string(), None),
            ]
        );
    }

    #[tokio::test]
    async fn test_passkey_add_generates_enroll_link_in_dm() {
        let (db, services) = setup().await;
        let account = services
            .accounts
            .create("evan", STATUS_ACTIVE)
            .await
            .unwrap();

        // In public room: refused
        let mut public_ctx = ctx("passkey", &account, false);
        public_ctx.raw_args = "add".into();
        let reply = module()
            .handle_command(&public_ctx, &services, &db)
            .await
            .unwrap()
            .html_body()
            .to_owned();
        assert!(reply.contains("direct message"));

        // In DM: generates ?enroll= token link
        let mut dm_ctx = ctx("passkey", &account, true);
        dm_ctx.raw_args = "add".into();
        let reply = module()
            .handle_command(&dm_ctx, &services, &db)
            .await
            .unwrap()
            .html_body()
            .to_owned();
        assert!(reply.contains("?enroll="));
        assert!(reply.contains("bot.mbm.one"));
    }

    #[tokio::test]
    async fn test_passkey_add_is_refused_once_the_account_has_a_passkey() {
        let (db, services) = setup().await;
        let account = services
            .accounts
            .create("evan", STATUS_ACTIVE)
            .await
            .unwrap();

        services
            .accounts
            .add_passkey(&account, "pk1", "{}", "MacBook")
            .await
            .unwrap();

        for (command, args) in [("passkey", "add"), ("passkey", "enroll"), ("web", "")] {
            let mut dm = ctx(command, &account, true);
            dm.raw_args = args.into();
            let reply = module()
                .handle_command(&dm, &services, &db)
                .await
                .unwrap()
                .html_body()
                .to_owned();

            assert!(
                !reply.contains("?enroll="),
                "`!{command} {args}` must not hand out a link: {reply}"
            );
            assert!(
                reply.contains("!recover"),
                "`!{command} {args}` must point at the fenced route: {reply}"
            );
        }

        let issued = repo::credentials::count_enrollment_tokens(&db.pool, &account)
            .await
            .unwrap();
        assert_eq!(issued, 0, "no token may be minted for a passkey account");
    }

    #[tokio::test]
    async fn test_unlink_changes_nothing_from_matrix() {
        let (db, services) = setup().await;
        let account = services
            .accounts
            .create("evan", STATUS_ACTIVE)
            .await
            .unwrap();

        repo::linking::link(&db.pool, &account, "@evan:mbm.one")
            .await
            .unwrap();
        repo::linking::link(&db.pool, &account, "@evan:other.org")
            .await
            .unwrap();

        let session = services
            .accounts
            .create_session(&account, None, None, None)
            .await
            .unwrap();

        for args in ["sessions", "matrix @evan:other.org"] {
            let mut dm = ctx("unlink", &account, true);
            dm.raw_args = args.into();
            let reply = module()
                .handle_command(&dm, &services, &db)
                .await
                .unwrap()
                .html_body()
                .to_owned();
            assert!(
                reply.contains("web app"),
                "`!unlink {args}` must point to the web app: {reply}"
            );
        }

        assert!(
            services
                .accounts
                .resolve_session(&session)
                .await
                .unwrap()
                .is_some(),
            "the web session must survive"
        );
        let links = services
            .identity
            .user_ids(&account, Channel::Matrix)
            .await
            .unwrap();
        assert_eq!(links.len(), 2, "the second identity must stay linked");
    }

    #[tokio::test]
    async fn test_passkeys_list_and_remove_flow() {
        let (db, services) = setup().await;
        let account = services
            .accounts
            .create("evan", STATUS_ACTIVE)
            .await
            .unwrap();

        // Initially no passkeys
        let list_ctx = ctx("passkeys", &account, true);
        let reply = module()
            .handle_command(&list_ctx, &services, &db)
            .await
            .unwrap()
            .html_body()
            .to_owned();
        assert!(reply.contains("no passkeys linked"));

        // Insert two passkeys and a matrix link
        repo::linking::link(&db.pool, &account, "@evan:mbm.one")
            .await
            .unwrap();
        services
            .accounts
            .add_passkey(&account, "pk1", "{}", "MacBook")
            .await
            .unwrap();
        services
            .accounts
            .add_passkey(&account, "pk2", "{}", "iPhone")
            .await
            .unwrap();

        // List passkeys
        let reply = module()
            .handle_command(&list_ctx, &services, &db)
            .await
            .unwrap()
            .html_body()
            .to_owned();
        assert!(reply.contains("MacBook"));
        assert!(reply.contains("iPhone"));

        // Removing a passkey via Matrix DM is safely rejected, directing user to web settings
        let mut remove_ctx = ctx("passkey", &account, true);
        remove_ctx.raw_args = "rm 1".into();
        let reply = module()
            .handle_command(&remove_ctx, &services, &db)
            .await
            .unwrap()
            .html_body()
            .to_owned();
        assert!(reply.contains("Passkey Removal Requires Verification"));
        assert!(reply.contains("/account"));

        // Verify passkeys were NOT deleted from database
        let creds_after = services.accounts.list_passkeys(&account).await.unwrap();
        assert_eq!(creds_after.len(), 2);
    }
}
