//! Chat-initiated account linking for every channel (spec §7.2, D1/D4).
//!
//! Proof of *identity* ownership is the direct conversation that minted the
//! token; proof of *account* ownership is the passkey session that claims it.
//! The token is shown once, in that conversation and the browser URL; only its
//! SHA-256 is stored, and no type here that holds it prints it.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use base64::Engine;
use chrono::{DateTime, Utc};
use sqlx::Connection;
use sqlx::PgConnection;
use sqlx::PgPool;
use sqlx::Postgres;
use sqlx::Transaction;

use webauthn_rs::prelude::Passkey;

use crate::application::accounts::merge::MergeService;
use crate::application::accounts::register_account_in;
use crate::application::AppError;
use crate::domain::accounts::{hash_token, AccountId, MergePreview, SecurityEventKind};
use crate::domain::channels::{
    decide_link, Channel, ConversationAddress, LinkDecision, LinkedIdentity, PlatformIdentity,
};
use crate::domain::events::SystemEvent;
use crate::infrastructure::postgres::repositories::accounts::{core, security_events};
use crate::infrastructure::postgres::repositories::link_tokens::{self, LinkTokenRow};
use crate::infrastructure::postgres::repositories::{identities, outbox};

/// 32 random bytes: far beyond guessing within a ten-minute window.
const TOKEN_BYTES: usize = 32;

/// A freshly minted token and the page that finishes the link.
pub struct LinkStart {
    pub token: String,
    pub url: String,
}

// Manual: the derived impl would print the token into any log that holds one.
impl fmt::Debug for LinkStart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LinkStart").finish_non_exhaustive()
    }
}

/// What the landing page should offer for a token and viewer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkPreview {
    Bind {
        identity: LinkedIdentity,
    },
    AlreadyYours,
    /// Claiming also absorbs another account; `preview` counts what moves.
    NeedsMerge {
        identity: LinkedIdentity,
        preview: MergePreview,
    },
    OwnedElsewhere,
    /// No viewer, and the identity may register a fresh account (D4).
    SignupAllowed {
        identity: LinkedIdentity,
    },
    /// No viewer, and the identity must sign in to an existing account.
    SignInRequired {
        identity: LinkedIdentity,
    },
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkOutcome {
    Linked { account: AccountId },
    AlreadyYours,
    Merged { account: AccountId, from: AccountId },
    OwnedElsewhere,
    Expired,
}

#[derive(Clone)]
pub struct LinkService {
    pool: PgPool,
    web_url: String,
    /// Identities that may register a new account from a link token (D4),
    /// per channel, in canonical form. Never logged.
    signup_allowlist: Arc<HashMap<Channel, Vec<String>>>,
    app_config: Option<crate::application::config::AppConfigService>,
}

impl LinkService {
    pub fn new(
        pool: PgPool,
        web_url: String,
        signup_allowlist: HashMap<Channel, Vec<String>>,
    ) -> Self {
        Self {
            pool,
            web_url,
            signup_allowlist: Arc::new(signup_allowlist),
            app_config: None,
        }
    }

    /// The same service with the deployment's web origin and signup allowlist.
    pub fn with_config(
        self,
        web_url: String,
        signup_allowlist: HashMap<Channel, Vec<String>>,
    ) -> Self {
        Self {
            pool: self.pool,
            web_url,
            signup_allowlist: Arc::new(signup_allowlist),
            app_config: self.app_config,
        }
    }

    /// Attaches the dynamic runtime configuration service for `registration.allow_link_signup`.
    pub fn with_app_config(
        mut self,
        app_config: crate::application::config::AppConfigService,
    ) -> Self {
        self.app_config = Some(app_config);
        self
    }

    /// Mints a token for `identity`, who asked in `conversation`. A shared
    /// conversation is refused before anything is minted: a token posted where
    /// others can read it would let a bystander claim the identity's link.
    /// Starting again burns the previous live token.
    pub async fn start(
        &self,
        identity: &PlatformIdentity,
        conversation: &ConversationAddress,
        is_direct: bool,
    ) -> Result<LinkStart, AppError> {
        if !is_direct {
            return Err(AppError::Validation(
                "A link can only be started in a direct conversation.".to_string(),
            ));
        }
        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(crate::utils::random::random_bytes(TOKEN_BYTES));
        let now = Utc::now();

        // "One live token per identity" is best effort under concurrent
        // starts: both tokens still belong to the same identity's DM.
        let mut tx = self.pool.begin().await?;
        link_tokens::consume_live_for_identity(&mut tx, identity, now).await?;
        link_tokens::insert(
            &mut tx,
            &hash_token(&token),
            identity,
            &conversation.to_storage(),
            now,
        )
        .await?;
        tx.commit().await?;

        tracing::info!(channel = %identity.channel(), "link token issued");
        let url = format!("{}/link?token={token}", self.web_url.trim_end_matches('/'));
        Ok(LinkStart { token, url })
    }

    /// What claiming `token` would do for `viewer` (`None`: not signed in).
    /// Reads only; the landing page shows this before anyone confirms.
    pub async fn inspect(
        &self,
        token: &str,
        viewer: Option<&AccountId>,
    ) -> Result<LinkPreview, AppError> {
        let Some(row) = link_tokens::find(&self.pool, &hash_token(token)).await? else {
            return Ok(LinkPreview::Expired);
        };
        if !is_live(&row, Utc::now()) {
            return Ok(LinkPreview::Expired);
        }
        let identity = &row.identity;
        let shown = shown_identity(identity);
        let mut conn = self.pool.acquire().await?;
        let current = holder(&mut conn, identity).await?;

        let Some(viewer) = viewer else {
            return Ok(match current {
                Some((_, false)) => LinkPreview::SignInRequired { identity: shown },
                _ if self.signup_allowed(identity) => {
                    LinkPreview::SignupAllowed { identity: shown }
                }
                _ => LinkPreview::SignInRequired { identity: shown },
            });
        };
        Ok(
            match decide_link(
                viewer,
                current.as_ref().map(|(id, unclaimed)| (id, *unclaimed)),
            ) {
                LinkDecision::Bind => match self
                    .previous_account_with_data(&mut conn, identity, viewer)
                    .await?
                {
                    Some((_, preview)) => LinkPreview::NeedsMerge {
                        identity: shown,
                        preview,
                    },
                    None => LinkPreview::Bind { identity: shown },
                },
                LinkDecision::AlreadyYours => LinkPreview::AlreadyYours,
                LinkDecision::Merge { from } => LinkPreview::NeedsMerge {
                    identity: shown,
                    preview: MergeService::preview(&self.pool, &from).await?,
                },
                LinkDecision::OwnedElsewhere => LinkPreview::OwnedElsewhere,
            },
        )
    }

    /// Binds (or merges, then binds) the token's identity to `account`, burns
    /// the token and publishes the confirmation, all in one transaction: a
    /// failure anywhere leaves the token usable and nothing changed.
    pub async fn claim(&self, token: &str, account: &AccountId) -> Result<LinkOutcome, AppError> {
        let mut tx = self.pool.begin().await?;
        let outcome = self.claim_in(&mut tx, token, account, false).await?;
        tx.commit().await?;
        Ok(outcome)
    }

    /// Whether `identity` may register a fresh account from a link token.
    pub fn signup_allowed(&self, identity: &PlatformIdentity) -> bool {
        if self
            .app_config
            .as_ref()
            .is_some_and(|cfg| !cfg.snapshot().registration_allow_link_signup())
        {
            return false;
        }
        self.signup_allowlist
            .get(&identity.channel())
            .is_some_and(|allowed| allowed.iter().any(|id| id == identity.user_id().as_str()))
    }

    /// The signup finish handler's claim, inside its passkey-registration
    /// transaction: the same steps as [`Self::claim`] on a savepoint of the
    /// caller's connection (the caller commits), plus the allowlist check.
    pub async fn claim_for_new_account(
        &self,
        conn: &mut PgConnection,
        token: &str,
        account: &AccountId,
    ) -> Result<LinkOutcome, AppError> {
        let mut savepoint = conn.begin().await?;
        let outcome = self.claim_in(&mut savepoint, token, account, true).await?;
        savepoint.commit().await?;
        Ok(outcome)
    }

    /// The signup finish: creates the account with its first passkey and binds
    /// the token's identity to it, all in one transaction. Anything but a
    /// completed bind (a refused allowlist, an expired or lost token, a failed
    /// write) rolls the new account and credential back with it.
    pub async fn sign_up_with_passkey(
        &self,
        token: &str,
        account: &AccountId,
        display_name: &str,
        passkey: &Passkey,
        label: &str,
    ) -> Result<LinkOutcome, AppError> {
        let mut tx = self.pool.begin().await?;
        register_account_in(&mut tx, account, display_name, passkey, label).await?;
        let outcome = self.claim_for_new_account(&mut tx, token, account).await?;
        if matches!(
            outcome,
            LinkOutcome::Linked { .. } | LinkOutcome::Merged { .. }
        ) {
            tx.commit().await?;
        }
        // Dropping the transaction rolls the account and passkey back.
        Ok(outcome)
    }

    async fn claim_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        token: &str,
        account: &AccountId,
        require_signup: bool,
    ) -> Result<LinkOutcome, AppError> {
        let hash = hash_token(token);
        let now = Utc::now();
        let Some(row) = link_tokens::lock(tx, &hash).await? else {
            return Ok(LinkOutcome::Expired);
        };
        if !is_live(&row, now) {
            return Ok(LinkOutcome::Expired);
        }
        let identity = &row.identity;
        if require_signup && !self.signup_allowed(identity) {
            return Err(AppError::Forbidden(
                "This account may not register here. Sign in to an existing account instead."
                    .to_string(),
            ));
        }

        let current = holder(tx, identity).await?;
        let mut merged_rows = 0;
        let outcome = match decide_link(account, current.as_ref().map(|(id, un)| (id, *un))) {
            LinkDecision::Bind => {
                if identities::bind(&mut **tx, account, identity).await? {
                    // A3: an identity once unlinked from an account that still
                    // holds data merges that account back; the preview the
                    // user confirmed with this claim said so.
                    match self
                        .previous_account_with_data(tx, identity, account)
                        .await?
                    {
                        Some((from, _)) => {
                            let moved = MergeService::merge_within(tx, &from, account).await?;
                            merged_rows = moved.moved.total();
                            LinkOutcome::Merged {
                                account: *account,
                                from,
                            }
                        }
                        None => LinkOutcome::Linked { account: *account },
                    }
                } else {
                    // Lost a race to another bind: someone holds it now.
                    LinkOutcome::OwnedElsewhere
                }
            }
            LinkDecision::AlreadyYours => LinkOutcome::AlreadyYours,
            LinkDecision::Merge { from } => {
                let moved = MergeService::merge_within(tx, &from, account).await?;
                merged_rows = moved.moved.total();
                // The merge already carried the identity (`platform_identities`
                // follows the absorbed account); this pins the result.
                identities::rebind(&mut **tx, account, identity).await?;
                LinkOutcome::Merged {
                    account: *account,
                    from,
                }
            }
            LinkDecision::OwnedElsewhere => LinkOutcome::OwnedElsewhere,
        };

        link_tokens::consume(tx, &hash, now).await?;
        let changed = matches!(
            outcome,
            LinkOutcome::Linked { .. } | LinkOutcome::Merged { .. }
        );
        if changed {
            let detail = format!("{}:{}", identity.channel(), identity.user_id().as_str());
            security_events::record(
                &mut **tx,
                account,
                SecurityEventKind::IdentityLinked,
                Some(&detail),
            )
            .await;
        }
        if matches!(outcome, LinkOutcome::Merged { .. }) {
            security_events::record(
                &mut **tx,
                account,
                SecurityEventKind::AccountsMerged,
                Some(&format!("{merged_rows} rows")),
            )
            .await;
        }
        if changed || outcome == LinkOutcome::AlreadyYours {
            outbox::publish(
                tx,
                &SystemEvent::IdentityLinked {
                    owner_id: *account,
                    channel: identity.channel(),
                    user_id: identity.user_id().as_str().to_owned(),
                    conversation: row.conversation.clone(),
                },
            )
            .await?;
        }
        tracing::info!(
            channel = %identity.channel(),
            account = %account,
            outcome = outcome_name(&outcome),
            "link token claimed"
        );
        Ok(outcome)
    }

    /// The account the identity was last unlinked from, when it still holds
    /// data (and is not the claimer).
    ///
    /// R165: Only unclaimed placeholder accounts (`STATUS_UNCLAIMED`) can be
    /// absorbed automatically. An active account (`STATUS_ACTIVE`) has its own
    /// user identity and credentials; re-linking an identity detached from it
    /// must never absorb its data.
    async fn previous_account_with_data(
        &self,
        conn: &mut PgConnection,
        identity: &PlatformIdentity,
        claimer: &AccountId,
    ) -> Result<Option<(AccountId, MergePreview)>, AppError> {
        let Some(previous) = security_events::last_unlinked_from(&mut *conn, identity).await?
        else {
            return Ok(None);
        };
        if previous == *claimer {
            return Ok(None);
        }
        let Some(account) = core::find(&mut *conn, &previous).await? else {
            return Ok(None);
        };
        if !account.is_unclaimed() {
            return Ok(None);
        }
        let preview = MergeService::preview(&self.pool, &previous).await?;
        Ok((preview.total() > 0).then_some((previous, preview)))
    }

    /// Purges expired or consumed link tokens older than `older_than`.
    pub async fn purge_expired(&self, older_than: std::time::Duration) -> Result<u64, AppError> {
        let chrono_dur =
            chrono::Duration::from_std(older_than).unwrap_or_else(|_| chrono::Duration::days(1));
        let cutoff = Utc::now() - chrono_dur;
        let purged = link_tokens::purge_expired(&self.pool, cutoff).await?;
        Ok(purged)
    }
}

fn is_live(row: &LinkTokenRow, now: DateTime<Utc>) -> bool {
    row.consumed_at.is_none() && row.expires_at > now
}

/// The identity as the landing page lists it: not linked yet, so "linked at"
/// is now.
fn shown_identity(identity: &PlatformIdentity) -> LinkedIdentity {
    LinkedIdentity::new(
        identity.channel(),
        identity.user_id().as_str().to_owned(),
        identity.display_name().map(str::to_owned),
        Utc::now(),
    )
}

/// The account an identity maps to now and whether it is unclaimed. A holder
/// whose row cannot be read counts as claimed: never merge what is unclear.
async fn holder(
    conn: &mut PgConnection,
    identity: &PlatformIdentity,
) -> Result<Option<(AccountId, bool)>, sqlx::Error> {
    let Some(id) = identities::find_account(&mut *conn, identity).await? else {
        return Ok(None);
    };
    let unclaimed = core::find(&mut *conn, &id)
        .await?
        .is_some_and(|account| account.is_unclaimed());
    Ok(Some((id, unclaimed)))
}

fn outcome_name(outcome: &LinkOutcome) -> &'static str {
    match outcome {
        LinkOutcome::Linked { .. } => "linked",
        LinkOutcome::AlreadyYours => "already_yours",
        LinkOutcome::Merged { .. } => "merged",
        LinkOutcome::OwnedElsewhere => "owned_elsewhere",
        LinkOutcome::Expired => "expired",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use sqlx::PgPool;

    use super::*;
    use crate::application::identity::IdentityService;
    use crate::domain::accounts::{hash_token, SecurityEventKind, STATUS_ACTIVE, STATUS_UNCLAIMED};
    use crate::domain::channels::{Channel, ConversationAddress, PlatformIdentity, PlatformUserId};
    use crate::domain::todos::{NewTodo, TodoPriority};
    use crate::infrastructure::postgres::repositories::accounts::{core, security_events};
    use crate::infrastructure::postgres::repositories::{
        identities, link_tokens, outbox, todos as todo_repo,
    };
    use crate::utils::time::now_at_storage_precision;

    fn ident(channel: Channel, raw: &str) -> PlatformIdentity {
        PlatformIdentity::new(channel, PlatformUserId::parse(channel, raw).unwrap(), None)
    }
    fn tg(raw: &str) -> PlatformIdentity {
        ident(Channel::Telegram, raw)
    }
    fn mx(raw: &str) -> PlatformIdentity {
        ident(Channel::Matrix, raw)
    }
    fn dm(id: i64) -> ConversationAddress {
        ConversationAddress::Telegram(id)
    }

    async fn setup_with(allowlist: HashMap<Channel, Vec<String>>) -> (LinkService, PgPool) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        (
            LinkService::new(pool.clone(), "https://bot.example".to_string(), allowlist),
            pool,
        )
    }
    async fn setup() -> (LinkService, PgPool) {
        setup_with(HashMap::new()).await
    }
    async fn account(pool: &PgPool, name: &str) -> AccountId {
        core::create(pool, name, STATUS_ACTIVE).await.unwrap()
    }
    async fn unclaimed_account(pool: &PgPool, name: &str) -> AccountId {
        core::create(pool, name, STATUS_UNCLAIMED).await.unwrap()
    }
    async fn add_todo(pool: &PgPool, owner: &AccountId, text: &str) -> i64 {
        todo_repo::create(
            pool,
            owner,
            &NewTodo {
                room_id: "!r:x".to_string(),
                sender_name: "evan".to_string(),
                text: text.to_string(),
                priority: TodoPriority::Medium,
                due_at: None,
                remind_before_minutes: None,
                remind_room_id: None,
            },
        )
        .await
        .unwrap()
        .id
    }
    async fn owner_of_identity(pool: &PgPool, who: &PlatformIdentity) -> Option<AccountId> {
        identities::find_account(pool, who).await.unwrap()
    }

    #[tokio::test]
    async fn start_refuses_a_shared_conversation_and_mints_nothing() {
        let (service, pool) = setup().await;
        let err = service
            .start(&tg("42"), &ConversationAddress::Telegram(-5), false)
            .await
            .expect_err("refused");
        assert!(matches!(err, AppError::Validation(_)), "{err:?}");
        assert_eq!(link_tokens::count(&pool).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn start_builds_the_landing_url_from_web_url_and_the_token() {
        let (service, _pool) = setup().await;
        let started = service.start(&tg("42"), &dm(42), true).await.unwrap();
        assert_eq!(
            started.url,
            format!("https://bot.example/link?token={}", started.token)
        );
        assert!(
            started.token.len() >= 43,
            "32 random bytes, base64url without padding"
        );
        assert!(!format!("{started:?}").contains(&started.token));
    }

    #[tokio::test]
    async fn a_token_is_stored_only_as_its_hash() {
        let (service, pool) = setup().await;
        let started = service.start(&tg("42"), &dm(42), true).await.unwrap();
        let hashes = link_tokens::all_hashes(&pool).await.unwrap();
        assert_eq!(hashes, vec![hash_token(&started.token)]);
        assert!(!hashes.contains(&started.token));
    }

    #[tokio::test]
    async fn starting_again_consumes_the_previous_token() {
        let (service, _pool) = setup().await;
        let t1 = service.start(&tg("42"), &dm(42), true).await.unwrap();
        let t2 = service.start(&tg("42"), &dm(42), true).await.unwrap();
        assert_ne!(t1.token, t2.token);
        assert_eq!(
            service.inspect(&t1.token, None).await.unwrap(),
            LinkPreview::Expired
        );
        assert!(!matches!(
            service.inspect(&t2.token, None).await.unwrap(),
            LinkPreview::Expired
        ));
    }

    #[tokio::test]
    async fn claim_binds_a_new_telegram_identity_and_publishes_identity_linked() {
        let (service, pool) = setup().await;
        let a = account(&pool, "a").await;
        let started = service.start(&tg("42"), &dm(42), true).await.unwrap();

        assert_eq!(
            service.claim(&started.token, &a).await.unwrap(),
            LinkOutcome::Linked { account: a }
        );
        assert_eq!(owner_of_identity(&pool, &tg("42")).await, Some(a));
        assert_eq!(
            outbox::event_types_for(&pool, &a).await.unwrap(),
            vec!["IdentityLinked"]
        );
        assert_eq!(
            outbox::delivery_destinations_for(&pool, &a).await.unwrap(),
            vec!["telegram:42"]
        );
        let events = security_events::list(&pool, &a, 10).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, SecurityEventKind::IdentityLinked.as_str());
        assert_eq!(events[0].detail.as_deref(), Some("telegram:42"));
    }

    #[tokio::test]
    async fn a_token_works_once() {
        let (service, pool) = setup().await;
        let a = account(&pool, "a").await;
        let b = account(&pool, "b").await;
        let started = service.start(&tg("42"), &dm(42), true).await.unwrap();
        service.claim(&started.token, &a).await.unwrap();
        assert_eq!(
            service.claim(&started.token, &b).await.unwrap(),
            LinkOutcome::Expired
        );
        assert_eq!(owner_of_identity(&pool, &tg("42")).await, Some(a));
    }

    #[tokio::test]
    async fn an_unknown_token_is_expired() {
        let (service, pool) = setup().await;
        let a = account(&pool, "a").await;
        assert_eq!(
            service.claim("not-a-token", &a).await.unwrap(),
            LinkOutcome::Expired
        );
    }

    #[tokio::test]
    async fn an_expired_token_binds_nothing() {
        let (service, pool) = setup().await;
        let a = account(&pool, "a").await;
        let long_ago = now_at_storage_precision() - chrono::Duration::minutes(11);
        let mut conn = pool.acquire().await.unwrap();
        link_tokens::insert(
            &mut conn,
            &hash_token("old"),
            &tg("42"),
            "telegram:42",
            long_ago,
        )
        .await
        .unwrap();
        drop(conn);

        assert_eq!(
            service.claim("old", &a).await.unwrap(),
            LinkOutcome::Expired
        );
        assert_eq!(
            service.inspect("old", Some(&a)).await.unwrap(),
            LinkPreview::Expired
        );
        assert_eq!(owner_of_identity(&pool, &tg("42")).await, None);
        assert!(outbox::event_types_for(&pool, &a).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_auto_provisioned_matrix_account_is_merged_into_the_claimer() {
        let (service, pool) = setup().await;
        let identity = IdentityService::new(pool.clone());
        let b = identity.provision_matrix_for_test("@evan:mbm.one").await;
        let todo = add_todo(&pool, &b, "history").await;
        let a = account(&pool, "a").await;

        let started = service
            .start(
                &mx("@evan:mbm.one"),
                &ConversationAddress::Matrix("!dm:x".into()),
                true,
            )
            .await
            .unwrap();
        match service.inspect(&started.token, Some(&a)).await.unwrap() {
            LinkPreview::NeedsMerge { preview, .. } => assert_eq!(preview.todos, 1),
            other => panic!("expected a merge preview, got {other:?}"),
        }
        assert_eq!(
            service.claim(&started.token, &a).await.unwrap(),
            LinkOutcome::Merged {
                account: a,
                from: b
            }
        );

        assert!(todo_repo::find_owned(&pool, &a, todo)
            .await
            .unwrap()
            .is_some());
        assert_eq!(
            owner_of_identity(&pool, &mx("@evan:mbm.one")).await,
            Some(a)
        );
        assert!(core::find(&pool, &b).await.unwrap().is_none());
        let kinds: Vec<_> = security_events::list(&pool, &a, 10)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect();
        assert!(kinds.contains(&"identity_linked".to_string()));
        assert!(kinds.contains(&"accounts_merged".to_string()));
        assert_eq!(
            outbox::delivery_destinations_for(&pool, &a).await.unwrap(),
            vec!["matrix:!dm:x"]
        );
    }

    #[tokio::test]
    async fn an_identity_owned_by_a_real_account_is_not_stolen() {
        let (service, pool) = setup().await;
        let c = account(&pool, "c").await;
        identities::bind(&pool, &c, &tg("42")).await.unwrap();
        let a = account(&pool, "a").await;
        let started = service.start(&tg("42"), &dm(42), true).await.unwrap();

        assert_eq!(
            service.inspect(&started.token, Some(&a)).await.unwrap(),
            LinkPreview::OwnedElsewhere
        );
        assert_eq!(
            service.claim(&started.token, &a).await.unwrap(),
            LinkOutcome::OwnedElsewhere
        );
        assert_eq!(owner_of_identity(&pool, &tg("42")).await, Some(c));
        assert!(outbox::event_types_for(&pool, &a).await.unwrap().is_empty());
        assert!(security_events::list(&pool, &a, 10)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn claiming_an_identity_already_on_my_account_is_a_quiet_success() {
        let (service, pool) = setup().await;
        let a = account(&pool, "a").await;
        identities::bind(&pool, &a, &tg("42")).await.unwrap();
        let started = service.start(&tg("42"), &dm(42), true).await.unwrap();

        assert!(matches!(
            service.inspect(&started.token, Some(&a)).await.unwrap(),
            LinkPreview::AlreadyYours
        ));
        assert_eq!(
            service.claim(&started.token, &a).await.unwrap(),
            LinkOutcome::AlreadyYours
        );
        assert_eq!(owner_of_identity(&pool, &tg("42")).await, Some(a));
        // The audit trail records changes only; the chat still gets its answer.
        assert!(security_events::list(&pool, &a, 10)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            outbox::event_types_for(&pool, &a).await.unwrap(),
            vec!["IdentityLinked"]
        );
    }

    #[tokio::test]
    async fn a_failed_claim_leaves_the_token_usable() {
        let (service, pool) = setup().await;
        let a = account(&pool, "a").await;
        let started = service.start(&tg("42"), &dm(42), true).await.unwrap();
        outbox::fail_every_publish(&pool).await;

        assert!(service.claim(&started.token, &a).await.is_err());

        assert!(matches!(
            service.inspect(&started.token, Some(&a)).await.unwrap(),
            LinkPreview::Bind { .. }
        ));
        assert_eq!(owner_of_identity(&pool, &tg("42")).await, None);
        assert!(security_events::list(&pool, &a, 10)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn signup_is_allowed_only_for_allowlisted_identities() {
        let (service, _pool) =
            setup_with(HashMap::from([(Channel::Telegram, vec!["1".to_string()])])).await;
        assert!(service.signup_allowed(&tg("1")));
        assert!(!service.signup_allowed(&tg("2")));
        assert!(!service.signup_allowed(&mx("@one:x")));
        assert!(!service.signup_allowed(&ident(Channel::Discord, "1")));
    }

    #[tokio::test]
    async fn inspect_without_a_viewer_offers_signup_only_to_the_allowlisted() {
        let (service, _pool) =
            setup_with(HashMap::from([(Channel::Telegram, vec!["1".to_string()])])).await;
        let allowed = service.start(&tg("1"), &dm(1), true).await.unwrap();
        let stranger = service.start(&tg("2"), &dm(2), true).await.unwrap();

        assert!(matches!(
            service.inspect(&allowed.token, None).await.unwrap(),
            LinkPreview::SignupAllowed { .. }
        ));
        assert!(matches!(
            service.inspect(&stranger.token, None).await.unwrap(),
            LinkPreview::SignInRequired { .. }
        ));
    }

    #[tokio::test]
    async fn inspect_without_a_viewer_never_offers_signup_for_a_claimed_identity() {
        let (service, pool) =
            setup_with(HashMap::from([(Channel::Telegram, vec!["1".to_string()])])).await;
        let c = account(&pool, "c").await;
        identities::bind(&pool, &c, &tg("1")).await.unwrap();
        let started = service.start(&tg("1"), &dm(1), true).await.unwrap();
        assert!(matches!(
            service.inspect(&started.token, None).await.unwrap(),
            LinkPreview::SignInRequired { .. }
        ));
    }

    /// A3: an identity unlinked from an unclaimed account that still holds data
    /// turns a plain bind into a merge prompt, under either audit kind.
    #[tokio::test]
    async fn a_previous_unlink_with_data_turns_a_bind_into_a_merge_prompt() {
        for (kind, detail) in [
            (SecurityEventKind::MatrixUnlinked, "@evan:mbm.one"),
            (SecurityEventKind::IdentityUnlinked, "matrix:@evan:mbm.one"),
        ] {
            let (service, pool) = setup().await;
            let old = unclaimed_account(&pool, "old").await;
            let todo = add_todo(&pool, &old, "history").await;
            security_events::record(&pool, &old, kind, Some(detail)).await;
            let new = account(&pool, "new").await;
            let started = service
                .start(
                    &mx("@evan:mbm.one"),
                    &ConversationAddress::Matrix("!dm:x".into()),
                    true,
                )
                .await
                .unwrap();

            match service.inspect(&started.token, Some(&new)).await.unwrap() {
                LinkPreview::NeedsMerge { preview, .. } => {
                    assert_eq!(preview.todos, 1, "{}", kind.as_str())
                }
                other => panic!("{}: expected NeedsMerge, got {other:?}", kind.as_str()),
            }

            // Claiming is the confirmation the preview asked for.
            assert_eq!(
                service.claim(&started.token, &new).await.unwrap(),
                LinkOutcome::Merged {
                    account: new,
                    from: old
                },
                "{}",
                kind.as_str()
            );
            assert!(todo_repo::find_owned(&pool, &new, todo)
                .await
                .unwrap()
                .is_some());
            assert_eq!(
                owner_of_identity(&pool, &mx("@evan:mbm.one")).await,
                Some(new)
            );
        }
    }

    /// R165: an identity unlinked from an active account must NOT merge its data
    /// into a new account upon re-linking; it must be a clean bind.
    #[tokio::test]
    async fn a_previous_unlink_from_an_active_account_does_not_merge() {
        let (service, pool) = setup().await;
        let old = account(&pool, "old").await; // active account
        let todo = add_todo(&pool, &old, "history").await;
        security_events::record(
            &pool,
            &old,
            SecurityEventKind::IdentityUnlinked,
            Some("matrix:@evan:mbm.one"),
        )
        .await;
        let new = account(&pool, "new").await;
        let started = service
            .start(
                &mx("@evan:mbm.one"),
                &ConversationAddress::Matrix("!dm:x".into()),
                true,
            )
            .await
            .unwrap();

        match service.inspect(&started.token, Some(&new)).await.unwrap() {
            LinkPreview::Bind { .. } => {}
            other => panic!("expected Bind, got {other:?}"),
        }

        assert_eq!(
            service.claim(&started.token, &new).await.unwrap(),
            LinkOutcome::Linked { account: new }
        );
        // Data stays with the old account
        assert!(todo_repo::find_owned(&pool, &old, todo)
            .await
            .unwrap()
            .is_some());
        assert!(todo_repo::find_owned(&pool, &new, todo)
            .await
            .unwrap()
            .is_none());
        assert!(core::find(&pool, &old).await.unwrap().is_some());
        assert_eq!(
            owner_of_identity(&pool, &mx("@evan:mbm.one")).await,
            Some(new)
        );
    }

    #[tokio::test]
    async fn a_previous_unlink_without_data_stays_a_plain_bind() {
        let (service, pool) = setup().await;
        let old = account(&pool, "old").await;
        security_events::record(
            &pool,
            &old,
            SecurityEventKind::IdentityUnlinked,
            Some("matrix:@evan:mbm.one"),
        )
        .await;
        let new = account(&pool, "new").await;
        let started = service
            .start(
                &mx("@evan:mbm.one"),
                &ConversationAddress::Matrix("!dm:x".into()),
                true,
            )
            .await
            .unwrap();
        assert!(matches!(
            service.inspect(&started.token, Some(&new)).await.unwrap(),
            LinkPreview::Bind { .. }
        ));
        assert_eq!(
            service.claim(&started.token, &new).await.unwrap(),
            LinkOutcome::Linked { account: new }
        );
        assert!(core::find(&pool, &old).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn claim_for_new_account_binds_an_allowlisted_identity_on_the_given_connection() {
        let (service, pool) =
            setup_with(HashMap::from([(Channel::Telegram, vec!["1".to_string()])])).await;
        let started = service.start(&tg("1"), &dm(1), true).await.unwrap();
        let fresh = account(&pool, "fresh").await;

        let mut tx = pool.begin().await.unwrap();
        let outcome = service
            .claim_for_new_account(&mut tx, &started.token, &fresh)
            .await
            .unwrap();
        assert_eq!(outcome, LinkOutcome::Linked { account: fresh });
        // The caller owns the transaction: rolling back undoes everything.
        tx.rollback().await.unwrap();
        assert_eq!(owner_of_identity(&pool, &tg("1")).await, None);
        assert!(matches!(
            service.inspect(&started.token, None).await.unwrap(),
            LinkPreview::SignupAllowed { .. }
        ));

        let mut tx = pool.begin().await.unwrap();
        service
            .claim_for_new_account(&mut tx, &started.token, &fresh)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(owner_of_identity(&pool, &tg("1")).await, Some(fresh));
        assert_eq!(
            outbox::event_types_for(&pool, &fresh).await.unwrap(),
            vec!["IdentityLinked"]
        );
    }

    #[tokio::test]
    async fn claim_for_new_account_refuses_an_identity_off_the_allowlist() {
        let (service, pool) = setup().await;
        let started = service.start(&tg("2"), &dm(2), true).await.unwrap();
        let fresh = account(&pool, "fresh").await;

        let mut tx = pool.begin().await.unwrap();
        let err = service
            .claim_for_new_account(&mut tx, &started.token, &fresh)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)), "{err:?}");
        tx.rollback().await.unwrap();
        assert_eq!(owner_of_identity(&pool, &tg("2")).await, None);
        assert!(matches!(
            service.inspect(&started.token, None).await.unwrap(),
            LinkPreview::SignInRequired { .. }
        ));
    }

    async fn count_credentials(pool: &PgPool, account: &AccountId) -> usize {
        crate::infrastructure::postgres::repositories::accounts::credentials::count_for_account(
            pool, account,
        )
        .await
        .unwrap() as usize
    }

    #[tokio::test]
    async fn signup_creates_the_account_passkey_and_link_together() {
        let (service, pool) =
            setup_with(HashMap::from([(Channel::Telegram, vec!["1".to_string()])])).await;
        let started = service.start(&tg("1"), &dm(1), true).await.unwrap();
        let fresh = AccountId::generate();

        let outcome = service
            .sign_up_with_passkey(
                &started.token,
                &fresh,
                "Evan",
                &crate::application::accounts::test_passkey(&[9, 9], 0),
                "Passkey",
            )
            .await
            .unwrap();

        assert_eq!(outcome, LinkOutcome::Linked { account: fresh });
        assert!(core::find(&pool, &fresh).await.unwrap().is_some());
        assert_eq!(count_credentials(&pool, &fresh).await, 1);
        assert_eq!(owner_of_identity(&pool, &tg("1")).await, Some(fresh));
        assert_eq!(
            outbox::event_types_for(&pool, &fresh).await.unwrap(),
            vec!["IdentityLinked"]
        );
    }

    #[tokio::test]
    async fn a_failed_link_rolls_the_new_account_and_credential_back() {
        let (service, pool) =
            setup_with(HashMap::from([(Channel::Telegram, vec!["1".to_string()])])).await;
        let started = service.start(&tg("1"), &dm(1), true).await.unwrap();
        outbox::fail_every_publish(&pool).await;
        let fresh = AccountId::generate();

        let result = service
            .sign_up_with_passkey(
                &started.token,
                &fresh,
                "Evan",
                &crate::application::accounts::test_passkey(&[8, 8], 0),
                "Passkey",
            )
            .await;

        assert!(result.is_err());
        assert!(core::find(&pool, &fresh).await.unwrap().is_none());
        assert_eq!(count_credentials(&pool, &fresh).await, 0);
        assert_eq!(owner_of_identity(&pool, &tg("1")).await, None);
        assert!(matches!(
            service.inspect(&started.token, None).await.unwrap(),
            LinkPreview::SignupAllowed { .. }
        ));
    }

    #[tokio::test]
    async fn signup_off_the_allowlist_or_with_a_dead_token_creates_nothing() {
        let (service, pool) = setup().await;
        let started = service.start(&tg("2"), &dm(2), true).await.unwrap();
        for token in [started.token.as_str(), "no-such-token"] {
            let fresh = AccountId::generate();
            let result = service
                .sign_up_with_passkey(
                    token,
                    &fresh,
                    "Evan",
                    &crate::application::accounts::test_passkey(&[7, 1], 0),
                    "Passkey",
                )
                .await;
            assert!(
                matches!(
                    result,
                    Err(AppError::Forbidden(_)) | Ok(LinkOutcome::Expired)
                ),
                "{result:?}"
            );
            assert!(core::find(&pool, &fresh).await.unwrap().is_none());
            assert_eq!(count_credentials(&pool, &fresh).await, 0);
        }
    }
}
