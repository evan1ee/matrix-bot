//! Rooms use cases: each account's cache of the Matrix rooms it shares with
//! the bot, and resolving the room an action is delivered to.
//!
//! Discovering rooms needs a live Matrix client, so it happens in the Matrix
//! adapter (`interface/channels/matrix/rooms.rs`), which hands the result here.

use sqlx::PgPool;
use tracing::warn;

use crate::application::identity::IdentityService;
use crate::application::settings::SettingsService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::channels::Channel;
use crate::domain::rooms::{self as domain, CachedRoom, JoinedRoom, RoomTarget, WEB_ROOM};
use crate::infrastructure::postgres::repositories::rooms as repo;

/// What the room picker shows: the cache, or — when caching a fresh
/// discovery failed — the discovery itself, not yet cached.
#[derive(Debug, PartialEq, Eq)]
pub enum PickerRooms {
    Cached(Vec<CachedRoom>),
    Uncached(Vec<JoinedRoom>),
}

#[derive(Clone)]
pub struct RoomService {
    pool: PgPool,
    settings: SettingsService,
    identity: IdentityService,
}

impl RoomService {
    pub fn new(pool: PgPool, settings: SettingsService, identity: IdentityService) -> Self {
        Self {
            pool,
            settings,
            identity,
        }
    }

    /// The account's cached rooms, in picker order.
    pub async fn cached_rooms(&self, account_id: &AccountId) -> Result<Vec<CachedRoom>, AppError> {
        Ok(repo::list_for_account(&self.pool, account_id).await?)
    }

    /// Replaces the account's cache with a fresh discovery and returns the
    /// cache as stored.
    ///
    /// One transaction: a failure part-way leaves the previous cache whole,
    /// never a mix of old and new rooms or an empty cache.
    pub async fn replace_cached(
        &self,
        account_id: &AccountId,
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

    /// Adopts a live discovery made because the cache was empty. Caching is
    /// an optimisation here, so a failure is logged and the discovery itself
    /// is the answer, in the same picker order the cache would give. An empty
    /// discovery leaves the cache alone.
    pub async fn adopt_discovery(
        &self,
        account_id: &AccountId,
        mut discovered: Vec<JoinedRoom>,
    ) -> PickerRooms {
        if discovered.is_empty() {
            return PickerRooms::Uncached(discovered);
        }
        match self.replace_cached(account_id, &discovered).await {
            Ok(saved) => PickerRooms::Cached(saved),
            Err(err) => {
                warn!(%account_id, error = %err, "Could not cache discovered rooms");
                discovered.sort_by(JoinedRoom::picker_order);
                PickerRooms::Uncached(discovered)
            }
        }
    }

    pub async fn is_cached(&self, account_id: &AccountId, room_id: &str) -> Result<bool, AppError> {
        Ok(repo::is_cached(&self.pool, account_id, room_id).await?)
    }

    /// Caches a room whose membership the caller has just proven live, so the
    /// next check takes the cached path.
    pub async fn remember(
        &self,
        account_id: &AccountId,
        room: &JoinedRoom,
    ) -> Result<(), AppError> {
        Ok(repo::insert(&self.pool, account_id, room).await?)
    }

    /// The Matrix identities linked to the account: a room is reachable for
    /// the account when one of them has joined it.
    pub async fn linked_mxids(&self, account_id: &AccountId) -> Result<Vec<String>, AppError> {
        self.identity.user_ids(account_id, Channel::Matrix).await
    }

    /// The room an action is delivered to when it names none; see
    /// [`domain::default_room`] for the priority order.
    ///
    /// Infallible by design: the result only picks where a message goes, so
    /// an input that cannot be read is logged and treated as absent, and
    /// resolution carries on with the others.
    pub async fn default_room(&self, account_id: &AccountId) -> Option<String> {
        let rooms = self.cached_rooms_or_empty(account_id).await;
        let mxids = logged_or_default(
            self.identity.user_ids(account_id, Channel::Matrix).await,
            "linked Matrix ids",
            account_id,
        );
        let dev_room_id = logged_or_default(
            self.settings.get(account_id).await.map(|s| s.dev_room_id),
            "dev room setting",
            account_id,
        );
        domain::default_room(&rooms, &mxids, dev_room_id.as_deref()).map(str::to_owned)
    }

    /// The room an action asked for by `query` (a room id, a room name, or
    /// nothing), falling back to [`WEB_ROOM`] when no Matrix room resolves.
    /// Infallible for the same reason as [`Self::default_room`].
    pub async fn resolve_target_room(&self, account_id: &AccountId, query: Option<&str>) -> String {
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

    async fn cached_rooms_or_empty(&self, account_id: &AccountId) -> Vec<CachedRoom> {
        logged_or_default(
            repo::list_for_account(&self.pool, account_id).await,
            "room cache",
            account_id,
        )
    }
}

/// Room resolution's degradation rule, in one place: log what could not be
/// read and carry on without it. Generic over the error type so it covers
/// both a repository's `sqlx::Error` and a service's `AppError` (settings).
fn logged_or_default<T: Default, E: std::fmt::Display>(
    result: Result<T, E>,
    input: &str,
    account_id: &AccountId,
) -> T {
    result.unwrap_or_else(|err| {
        warn!(%account_id, error = %err, "Resolving a room without the {input}");
        T::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::accounts::AccountService;
    use crate::domain::settings::UserSettings;

    async fn service_with_account(account_id: &AccountId) -> RoomService {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let accounts = AccountService::new(pool.clone());
        accounts
            .create_with_id(account_id, "Tester", crate::domain::accounts::STATUS_ACTIVE)
            .await
            .unwrap();
        RoomService::new(
            pool.clone(),
            SettingsService::new(pool.clone()),
            IdentityService::new(pool),
        )
    }

    fn room(room_id: &str, name: &str, is_direct: bool) -> JoinedRoom {
        JoinedRoom::new(room_id, Some(name), is_direct)
    }

    fn ids(rooms: &[CachedRoom]) -> Vec<&str> {
        rooms.iter().map(|c| c.room().room_id()).collect()
    }

    #[tokio::test]
    async fn replacing_the_cache_drops_rooms_no_longer_discovered() {
        let acct = AccountId::from_seed("acct_a");
        let service = service_with_account(&acct).await;
        service
            .replace_cached(&acct, &[room("!old:x", "old", false)])
            .await
            .unwrap();

        let saved = service
            .replace_cached(&acct, &[room("!new:x", "new", false)])
            .await
            .unwrap();

        assert_eq!(ids(&saved), ["!new:x"]);
    }

    #[tokio::test]
    async fn a_failed_replacement_keeps_the_previous_cache_whole() {
        let acct = AccountId::from_seed("acct_a");
        let service = service_with_account(&acct).await;
        service
            .replace_cached(&acct, &[room("!kept:x", "kept", false)])
            .await
            .unwrap();

        // PostgreSQL rejects a NUL byte in TEXT, so the second insert fails
        // after the delete and the first insert have already run.
        let result = service
            .replace_cached(
                &acct,
                &[room("!new:x", "new", false), room("!bad\0:x", "bad", false)],
            )
            .await;

        assert!(matches!(result, Err(AppError::Database(_))));
        let cached = service.cached_rooms(&acct).await.unwrap();
        assert_eq!(ids(&cached), ["!kept:x"]);
    }

    #[tokio::test]
    async fn an_adopted_discovery_is_cached() {
        let acct = AccountId::from_seed("acct_a");
        let service = service_with_account(&acct).await;
        let adopted = service
            .adopt_discovery(
                &acct,
                vec![room("!g:x", "general", false), room("!dm:x", "evan", true)],
            )
            .await;

        let PickerRooms::Cached(saved) = adopted else {
            panic!("expected the discovery to be cached: {adopted:?}");
        };
        assert_eq!(ids(&saved), ["!dm:x", "!g:x"]);
    }

    #[tokio::test]
    async fn a_discovery_that_cannot_be_cached_is_returned_in_picker_order() {
        let acct = AccountId::from_seed("acct_a");
        let service = service_with_account(&acct).await;
        service.pool.close().await;

        let adopted = service
            .adopt_discovery(
                &acct,
                vec![room("!g:x", "general", false), room("!dm:x", "evan", true)],
            )
            .await;

        assert_eq!(
            adopted,
            PickerRooms::Uncached(vec![
                room("!dm:x", "evan", true),
                room("!g:x", "general", false)
            ])
        );
    }

    #[tokio::test]
    async fn an_empty_discovery_leaves_the_cache_alone() {
        let acct = AccountId::from_seed("acct_a");
        let service = service_with_account(&acct).await;
        service
            .replace_cached(&acct, &[room("!kept:x", "kept", false)])
            .await
            .unwrap();

        let adopted = service.adopt_discovery(&acct, Vec::new()).await;

        assert_eq!(adopted, PickerRooms::Uncached(Vec::new()));
        assert_eq!(
            ids(&service.cached_rooms(&acct).await.unwrap()),
            ["!kept:x"]
        );
    }

    #[tokio::test]
    async fn a_remembered_room_is_cached() {
        let acct = AccountId::from_seed("acct_a");
        let service = service_with_account(&acct).await;
        assert!(!service.is_cached(&acct, "!r:x").await.unwrap());

        service
            .remember(&acct, &room("!r:x", "r", false))
            .await
            .unwrap();

        assert!(service.is_cached(&acct, "!r:x").await.unwrap());
    }

    #[tokio::test]
    async fn targets_resolve_against_the_cache() {
        let acct = AccountId::from_seed("acct_a");
        let service = service_with_account(&acct).await;
        assert_eq!(service.resolve_target_room(&acct, None).await, WEB_ROOM);

        service
            .replace_cached(
                &acct,
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
                service.resolve_target_room(&acct, query).await,
                expected,
                "{query:?}"
            );
        }
    }

    #[tokio::test]
    async fn the_default_room_reads_linked_ids_and_settings() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let identity = IdentityService::new(pool.clone());
        let account = identity.provision_matrix_for_test("@evan:mbm.one").await;
        let settings_service = SettingsService::new(pool.clone());
        let service = RoomService::new(pool.clone(), settings_service.clone(), identity);
        let settings = UserSettings {
            dev_room_id: Some("!dev:x".to_string()),
            ..UserSettings::default()
        };
        settings_service.save(&account, settings).await.unwrap();
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
        let acct = AccountId::from_seed("acct_a");
        let service = service_with_account(&acct).await;
        service.pool.close().await;

        assert_eq!(service.default_room(&acct).await, None);
        assert_eq!(service.resolve_target_room(&acct, None).await, WEB_ROOM);
    }
}
