//! Rooms use cases: each account's cache of the Matrix rooms it shares with
//! the bot, and resolving the room an action is delivered to.
//!
//! Discovering rooms needs a live Matrix client, so it happens in the Matrix
//! adapter (`interface/chat/rooms.rs`), which hands the result here.

use sqlx::PgPool;
use tracing::warn;

use crate::accounts::AccountRepository;
use crate::application::AppError;
use crate::domain::rooms::{self as domain, CachedRoom, JoinedRoom, RoomTarget, WEB_ROOM};
use crate::infrastructure::postgres::repositories::rooms as repo;
use crate::modules::settings::repository::SettingsRepository;

#[derive(Clone)]
pub struct RoomService {
    pool: PgPool,
}

impl RoomService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The account's cached rooms, in picker order.
    pub async fn cached_rooms(&self, account_id: &str) -> Result<Vec<CachedRoom>, AppError> {
        Ok(repo::list_for_account(&self.pool, account_id).await?)
    }

    /// Replaces the account's cache with a fresh discovery and returns the
    /// cache as stored.
    ///
    /// One transaction: a failure part-way leaves the previous cache whole,
    /// never a mix of old and new rooms or an empty cache.
    pub async fn replace_cached(
        &self,
        account_id: &str,
        rooms: &[JoinedRoom],
    ) -> Result<Vec<CachedRoom>, AppError> {
        let mut tx = self.pool.begin().await?;
        repo::delete_for_account(&mut *tx, account_id).await?;
        for room in rooms {
            repo::insert(&mut *tx, account_id, room).await?;
        }
        tx.commit().await?;
        self.cached_rooms(account_id).await
    }

    pub async fn is_cached(&self, account_id: &str, room_id: &str) -> Result<bool, AppError> {
        Ok(repo::is_cached(&self.pool, account_id, room_id).await?)
    }

    /// Caches a room whose membership the caller has just proven live, so the
    /// next check takes the cached path.
    pub async fn remember(&self, account_id: &str, room: &JoinedRoom) -> Result<(), AppError> {
        Ok(repo::insert(&self.pool, account_id, room).await?)
    }

    /// The Matrix identities linked to the account: a room is reachable for
    /// the account when one of them has joined it.
    pub async fn linked_mxids(&self, account_id: &str) -> Result<Vec<String>, AppError> {
        Ok(AccountRepository::linked_mxids(&self.pool, account_id).await?)
    }

    /// The room an action is delivered to when it names none; see
    /// [`domain::default_room`] for the priority order.
    ///
    /// Infallible by design: the result only picks where a message goes, so
    /// an input that cannot be read is logged and treated as absent, and
    /// resolution carries on with the others.
    pub async fn default_room(&self, account_id: &str) -> Option<String> {
        let rooms = self.cached_rooms_or_empty(account_id).await;
        let mxids = logged_or_default(
            AccountRepository::linked_mxids(&self.pool, account_id).await,
            "linked Matrix ids",
            account_id,
        );
        let dev_room_id = logged_or_default(
            SettingsRepository::get(&self.pool, account_id)
                .await
                .map(|settings| settings.dev_room_id),
            "dev room setting",
            account_id,
        );
        domain::default_room(&rooms, &mxids, dev_room_id.as_deref()).map(str::to_owned)
    }

    /// The room an action asked for by `query` (a room id, a room name, or
    /// nothing), falling back to [`WEB_ROOM`] when no Matrix room resolves.
    /// Infallible for the same reason as [`Self::default_room`].
    pub async fn resolve_target_room(&self, account_id: &str, query: Option<&str>) -> String {
        match RoomTarget::parse(query) {
            RoomTarget::RoomId(room_id) => room_id.to_owned(),
            RoomTarget::Named(name) => {
                let rooms = self.cached_rooms_or_empty(account_id).await;
                domain::match_named(name, &rooms).to_owned()
            }
            RoomTarget::UserDefault => self
                .default_room(account_id)
                .await
                .unwrap_or_else(|| WEB_ROOM.to_owned()),
        }
    }

    async fn cached_rooms_or_empty(&self, account_id: &str) -> Vec<CachedRoom> {
        logged_or_default(
            repo::list_for_account(&self.pool, account_id).await,
            "room cache",
            account_id,
        )
    }
}

/// Room resolution's degradation rule, in one place: log what could not be
/// read and carry on without it.
fn logged_or_default<T: Default>(
    result: Result<T, sqlx::Error>,
    input: &str,
    account_id: &str,
) -> T {
    result.unwrap_or_else(|err| {
        warn!(account_id, error = %err, "Resolving a room without the {input}");
        T::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::settings::models::UserSettings;

    async fn service_with_account(account_id: &str) -> RoomService {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        AccountRepository::create_with_id(
            &pool,
            account_id,
            "Tester",
            crate::accounts::STATUS_ACTIVE,
        )
        .await
        .unwrap();
        RoomService::new(pool)
    }

    fn room(room_id: &str, name: &str, is_direct: bool) -> JoinedRoom {
        JoinedRoom::new(room_id, Some(name), is_direct)
    }

    fn ids(rooms: &[CachedRoom]) -> Vec<&str> {
        rooms.iter().map(|c| c.room.room_id()).collect()
    }

    #[tokio::test]
    async fn replacing_the_cache_drops_rooms_no_longer_discovered() {
        let service = service_with_account("acct_a").await;
        service
            .replace_cached("acct_a", &[room("!old:x", "old", false)])
            .await
            .unwrap();

        let saved = service
            .replace_cached("acct_a", &[room("!new:x", "new", false)])
            .await
            .unwrap();

        assert_eq!(ids(&saved), ["!new:x"]);
    }

    #[tokio::test]
    async fn a_failed_replacement_keeps_the_previous_cache_whole() {
        let service = service_with_account("acct_a").await;
        service
            .replace_cached("acct_a", &[room("!kept:x", "kept", false)])
            .await
            .unwrap();

        // PostgreSQL rejects a NUL byte in TEXT, so the second insert fails
        // after the delete and the first insert have already run.
        let result = service
            .replace_cached(
                "acct_a",
                &[room("!new:x", "new", false), room("!bad\0:x", "bad", false)],
            )
            .await;

        assert!(matches!(result, Err(AppError::Database(_))));
        let cached = service.cached_rooms("acct_a").await.unwrap();
        assert_eq!(ids(&cached), ["!kept:x"]);
    }

    #[tokio::test]
    async fn a_remembered_room_is_cached() {
        let service = service_with_account("acct_a").await;
        assert!(!service.is_cached("acct_a", "!r:x").await.unwrap());

        service
            .remember("acct_a", &room("!r:x", "r", false))
            .await
            .unwrap();

        assert!(service.is_cached("acct_a", "!r:x").await.unwrap());
    }

    #[tokio::test]
    async fn targets_resolve_against_the_cache() {
        let service = service_with_account("acct_a").await;
        assert_eq!(service.resolve_target_room("acct_a", None).await, WEB_ROOM);

        service
            .replace_cached(
                "acct_a",
                &[
                    room("!direct:x", "Direct Chat", true),
                    room("!dev:x", "Dev Channel", false),
                ],
            )
            .await
            .unwrap();

        for (query, expected) in [
            (None, "!direct:x"),
            (Some("web"), "!direct:x"),
            (Some("!arbitrary:x"), "!arbitrary:x"),
            (Some("Dev Channel"), "!dev:x"),
            (Some("dev"), "!dev:x"),
            (Some("ops"), "ops"),
        ] {
            assert_eq!(
                service.resolve_target_room("acct_a", query).await,
                expected,
                "{query:?}"
            );
        }
    }

    #[tokio::test]
    async fn the_default_room_reads_linked_ids_and_settings() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let account = AccountRepository::resolve_or_provision(&pool, "@evan:mbm.one")
            .await
            .unwrap();
        let service = RoomService::new(pool.clone());
        let settings = UserSettings {
            dev_room_id: Some("!dev:x".to_string()),
            ..UserSettings::default()
        };
        SettingsRepository::upsert(&pool, &account, &settings)
            .await
            .unwrap();
        assert_eq!(
            service.default_room(&account).await.as_deref(),
            Some("!dev:x")
        );

        service
            .replace_cached(&account, &[room("!evan:x", "Evan", false)])
            .await
            .unwrap();
        assert_eq!(
            service.default_room(&account).await.as_deref(),
            Some("!evan:x")
        );
    }

    #[tokio::test]
    async fn an_unreadable_database_resolves_to_the_web_room() {
        let service = service_with_account("acct_a").await;
        service.pool.close().await;

        assert_eq!(service.default_room("acct_a").await, None);
        assert_eq!(service.resolve_target_room("acct_a", None).await, WEB_ROOM);
    }
}
