//! Application service for account merging: orchestration, preview, and transaction boundary.

use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::accounts::{
    AccountId, MergeOutcome, MergePreview, ACCOUNT_ID_DISCARDED, ACCOUNT_ID_FOLLOWS, OWNED_TABLES,
};
use crate::infrastructure::postgres::merge as repo_merge;

pub struct MergeService;

impl MergeService {
    /// Counts what a merge from `from` would move.
    pub async fn preview(pool: &PgPool, from: &AccountId) -> Result<MergePreview, AppError> {
        Ok(repo_merge::preview(pool, from).await?)
    }

    /// Moves everything `from` owns to `into`, then deletes `from`.
    ///
    /// Refuses to merge an account into itself.
    pub async fn merge(
        pool: &PgPool,
        from: &AccountId,
        into: &AccountId,
    ) -> Result<MergeOutcome, AppError> {
        if from == into {
            return Ok(MergeOutcome {
                moved: MergePreview::default(),
                discarded: 0,
            });
        }

        let mut tx = pool.begin().await?;
        let outcome = Self::merge_within(&mut tx, from, into).await?;
        tx.commit().await?;

        Ok(outcome)
    }

    /// The merge itself, against an open transaction.
    ///
    /// Separate from [`Self::merge`] so a test can run it and roll back, which
    /// is the only way to assert that a merge leaves nothing behind when it
    /// cannot finish.
    pub async fn merge_within(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        from: &AccountId,
        into: &AccountId,
    ) -> Result<MergeOutcome, AppError> {
        let mut moved = MergePreview::default();
        let mut discarded = 0i64;

        for &table in OWNED_TABLES {
            let updated = repo_merge::move_rows(tx, table, "owner_id", from, into).await?;
            moved.set(table, updated);

            let leftover = repo_merge::delete_leftovers(tx, table, "owner_id", from).await?;
            discarded += leftover;
        }

        // The audit ledger cannot go through the generic loop: it is immutable by
        // trigger and every hash is bound to the owner, so it needs the routine
        // that knows how to rebuild it.
        let ledger_rows =
            crate::infrastructure::postgres::repositories::portfolio::audit::absorb_ledger(
                tx, from, into,
            )
            .await?;
        moved.set("portfolio_audit_log", ledger_rows);

        // Anything else pointing at the absorbed account follows it
        for &table in ACCOUNT_ID_FOLLOWS {
            repo_merge::move_rows(tx, table, "account_id", from, into).await?;
            let leftover = repo_merge::delete_leftovers(tx, table, "account_id", from).await?;
            discarded += leftover;
        }

        // Credentials and ceremonies do NOT follow
        for &table in ACCOUNT_ID_DISCARDED {
            repo_merge::discard_account_table(tx, table, from).await?;
        }

        repo_merge::delete_account(tx, from).await?;

        Ok(MergeOutcome { moved, discarded })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::{STATUS_ACTIVE, STATUS_UNCLAIMED};
    use crate::infrastructure::postgres::repositories::accounts::core as repo_core;
    use crate::infrastructure::postgres::repositories::accounts::sessions as repo_sessions;

    async fn db() -> PgPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    async fn add_todo(pool: &PgPool, owner: &AccountId, text: &str) {
        sqlx::query!(
            "INSERT INTO todos (room_id, owner_id, sender_name, text, priority, completed) \
             VALUES ('!r:x', $1, 'someone', $2, 'medium', FALSE)",
            owner.as_uuid(),
            text
        )
        .execute(pool)
        .await
        .unwrap();
    }

    async fn owner_of(pool: &PgPool, text: &str) -> AccountId {
        let u: uuid::Uuid = sqlx::query_scalar!(
            r#"SELECT owner_id AS "owner_id: uuid::Uuid" FROM todos WHERE text = $1"#,
            text
        )
        .fetch_one(pool)
        .await
        .unwrap();
        AccountId::from_uuid(u)
    }

    #[tokio::test]
    async fn test_preview_counts_what_would_move() {
        let pool = db().await;
        let from = repo_core::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();

        add_todo(&pool, &from, "one").await;
        add_todo(&pool, &from, "two").await;
        sqlx::query!(
            "INSERT INTO user_settings (owner_id, timezone, currency) VALUES ($1, 'UTC', 'USD')",
            from.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();

        let preview = MergeService::preview(&pool, &from).await.unwrap();
        assert_eq!(preview.todos, 2);
        assert_eq!(preview.user_settings, 1);
        assert_eq!(preview.total(), 3);
        assert!(!preview.is_empty());
    }

    #[tokio::test]
    async fn test_merge_moves_the_rows_and_removes_the_source() {
        let pool = db().await;
        let from = repo_core::create(&pool, "matrix", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = repo_core::create(&pool, "web", STATUS_ACTIVE)
            .await
            .unwrap();

        add_todo(&pool, &from, "from matrix").await;
        add_todo(&pool, &into, "from the web").await;

        let outcome = MergeService::merge(&pool, &from, &into).await.unwrap();
        assert_eq!(outcome.moved.todos, 1);
        assert_eq!(outcome.discarded, 0);

        assert_eq!(owner_of(&pool, "from matrix").await, into);
        assert_eq!(owner_of(&pool, "from the web").await, into);

        assert!(repo_core::find(&pool, &from).await.unwrap().is_none());
        assert!(repo_core::find(&pool, &into).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_merge_carries_the_matrix_link_over() {
        let pool = db().await;
        let from = repo_core::resolve_or_provision(&pool, "@evan:mbm.one")
            .await
            .unwrap();
        let into = repo_core::create(&pool, "web", STATUS_ACTIVE)
            .await
            .unwrap();

        MergeService::merge(&pool, &from, &into).await.unwrap();

        assert_eq!(
            repo_core::find_by_mxid(&pool, "@evan:mbm.one")
                .await
                .unwrap(),
            Some(into)
        );
        assert_eq!(
            repo_core::linked_mxids(&pool, &into).await.unwrap(),
            vec!["@evan:mbm.one".to_string()]
        );
    }

    #[tokio::test]
    async fn test_conflicting_singleton_rows_keep_the_destinations() {
        let pool = db().await;
        let from = repo_core::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = repo_core::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();

        for (owner, tz) in [(&from, "Asia/Tokyo"), (&into, "Europe/Berlin")] {
            sqlx::query!(
                "INSERT INTO user_settings (owner_id, timezone, currency) VALUES ($1, $2, 'USD')",
                owner.as_uuid(),
                tz
            )
            .execute(&pool)
            .await
            .unwrap();
        }

        let outcome = MergeService::merge(&pool, &from, &into).await.unwrap();
        assert_eq!(outcome.discarded, 1, "the source's settings row is dropped");

        let tz = sqlx::query_scalar!(
            "SELECT timezone FROM user_settings WHERE owner_id = $1",
            into.as_uuid()
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(tz, "Europe/Berlin", "the surviving account's choice wins");

        let rows = sqlx::query_scalar!("SELECT COUNT(*) FROM user_settings")
            .fetch_one(&pool)
            .await
            .unwrap()
            .unwrap_or(0);
        assert_eq!(rows, 1, "the absorbed row must not be stranded");
    }

    #[tokio::test]
    async fn test_the_absorbed_accounts_sessions_are_revoked() {
        let pool = db().await;
        let from = repo_core::create(&pool, "old", STATUS_ACTIVE)
            .await
            .unwrap();
        let into = repo_core::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();

        let old_session = repo_sessions::create(&pool, &from, None, None, None)
            .await
            .unwrap();
        let kept_session = repo_sessions::create(&pool, &into, None, None, None)
            .await
            .unwrap();

        MergeService::merge(&pool, &from, &into).await.unwrap();

        assert!(
            repo_sessions::resolve(&pool, &old_session)
                .await
                .unwrap()
                .is_none(),
            "the absorbed account's session must not become a session on the survivor"
        );
        assert!(repo_sessions::resolve(&pool, &kept_session)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn test_self_merge_is_refused() {
        let pool = db().await;
        let account = repo_core::create(&pool, "solo", STATUS_ACTIVE)
            .await
            .unwrap();
        add_todo(&pool, &account, "keep me").await;

        let outcome = MergeService::merge(&pool, &account, &account)
            .await
            .unwrap();
        assert_eq!(outcome.moved.total(), 0);

        assert!(repo_core::find(&pool, &account).await.unwrap().is_some());
        assert_eq!(owner_of(&pool, "keep me").await, account);
    }

    #[tokio::test]
    async fn test_merging_twice_is_harmless() {
        let pool = db().await;
        let from = repo_core::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = repo_core::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();
        add_todo(&pool, &from, "only me").await;

        MergeService::merge(&pool, &from, &into).await.unwrap();
        let second = MergeService::merge(&pool, &from, &into).await.unwrap();

        assert_eq!(second.moved.total(), 0);
        assert_eq!(owner_of(&pool, "only me").await, into);
    }

    #[tokio::test]
    async fn test_no_owned_table_is_left_behind() {
        let pool = db().await;
        let from = repo_core::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = repo_core::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();

        // Put one row in every owned table
        add_todo(&pool, &from, "todo").await;
        sqlx::query!("INSERT INTO reminders (room_id, owner_id, sender_name, message, trigger_at) VALUES ('!r:x', $1, 'a', 'm', CURRENT_TIMESTAMP)", from.as_uuid()).execute(&pool).await.unwrap();
        sqlx::query!("INSERT INTO transactions (room_id, owner_id, sender_name, direction, amount_minor, currency, occurred_at) VALUES ('web', $1, 'a', 'expense', 100, 'USD', CURRENT_TIMESTAMP)", from.as_uuid()).execute(&pool).await.unwrap();
        sqlx::query!("INSERT INTO recurring_bills (room_id, owner_id, sender_name, name, category, amount_minor, currency, next_due_at) VALUES ('web', $1, 'a', 'rent', 'home', 100, 'USD', CURRENT_TIMESTAMP)", from.as_uuid()).execute(&pool).await.unwrap();
        sqlx::query!("INSERT INTO portfolio_holdings (owner_id, symbol, quantity_minor, avg_cost_minor, currency) VALUES ($1, 'AAPL', 100000000, 15000, 'USD')", from.as_uuid()).execute(&pool).await.unwrap();
        sqlx::query!("INSERT INTO portfolio_cash_balances (owner_id, currency, balance_minor) VALUES ($1, 'USD', 5000)", from.as_uuid()).execute(&pool).await.unwrap();
        sqlx::query!("INSERT INTO portfolio_daily_snapshots (owner_id, snapshot_date, currency, holdings_value_minor, cash_minor, total_net_worth_minor) VALUES ($1, '2026-09-06', 'USD', 100, 100, 200)", from.as_uuid()).execute(&pool).await.unwrap();
        sqlx::query!("INSERT INTO portfolio_price_alerts (owner_id, symbol, target_price_minor, currency, condition) VALUES ($1, 'AAPL', 20000, 'USD', 'ABOVE')", from.as_uuid()).execute(&pool).await.unwrap();
        sqlx::query!(
            "INSERT INTO finance_categories (owner_id, name) VALUES ($1, 'food')",
            from.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!(
            "INSERT INTO user_settings (owner_id, timezone, currency) VALUES ($1, 'UTC', 'USD')",
            from.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!("INSERT INTO bookmarks (owner_id, url, title, domain) VALUES ($1, 'https://example.com', 'Example', 'example.com')", from.as_uuid()).execute(&pool).await.unwrap();
        let habit_id = sqlx::query_scalar!("INSERT INTO habits (owner_id, name, frequency) VALUES ($1, 'Exercise', 'daily') RETURNING id", from.as_uuid()).fetch_one(&pool).await.unwrap();
        sqlx::query!(
            "INSERT INTO habit_logs (habit_id, owner_id, log_date) VALUES ($1, $2, '2026-09-06')",
            habit_id,
            from.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!("INSERT INTO life_day_logs (owner_id, log_date, mood) VALUES ($1, '2026-09-06', 'happy')", from.as_uuid()).execute(&pool).await.unwrap();
        sqlx::query!("INSERT INTO life_profiles (owner_id, birth_date, expected_lifespan_years) VALUES ($1, '1990-01-01', 90)", from.as_uuid()).execute(&pool).await.unwrap();
        sqlx::query!("INSERT INTO footprints (owner_id, title, latitude, longitude, arrival_date) VALUES ($1, 'Paris', 48.8566, 2.3522, '2026-09-06')", from.as_uuid()).execute(&pool).await.unwrap();
        sqlx::query!(
            "INSERT INTO ai_conversations (id, owner_id, room_id) VALUES ('conv1', $1, '!r:x')",
            from.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!(
            "INSERT INTO ai_user_configs (owner_id, model_name) VALUES ($1, 'gemini')",
            from.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!("INSERT INTO webhooks (id, owner_id, name, token_hash, token_prefix, default_room_id) VALUES ('wh1', $1, 'hook', 'thash', 'pref', '!r:x')", from.as_uuid()).execute(&pool).await.unwrap();
        sqlx::query!(
            "INSERT INTO finance_shares (owner_id, token, is_enabled) VALUES ($1, 'tok1', TRUE)",
            from.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!("INSERT INTO bot_message_targets (event_id, room_id, entity_type, entity_id, owner_id, title) VALUES ('ev1', '!r:x', 'todo', 1, $1, 'a todo')", from.as_uuid()).execute(&pool).await.unwrap();

        let preview = MergeService::preview(&pool, &from).await.unwrap();
        assert_eq!(preview.total(), 21, "one row per owned table");

        MergeService::merge(&pool, &from, &into).await.unwrap();

        for &table in OWNED_TABLES {
            let stranded = repo_merge::count_rows(&pool, table, "owner_id", &from)
                .await
                .unwrap();
            assert_eq!(
                stranded, 0,
                "{table} still has rows owned by the absorbed account"
            );

            let arrived = repo_merge::count_rows(&pool, table, "owner_id", &into)
                .await
                .unwrap();
            assert_eq!(arrived, 1, "{table} did not receive its row");
        }
    }

    #[tokio::test]
    async fn test_a_merge_moves_the_finance_share_and_reaction_targets() {
        let pool = db().await;
        let from = repo_core::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = repo_core::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();

        sqlx::query!(
            "INSERT INTO finance_shares (owner_id, token, is_enabled) VALUES ($1, 'tok123', TRUE)",
            from.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query!(
            "INSERT INTO bot_message_targets (event_id, room_id, entity_type, entity_id, owner_id, title) \
             VALUES ('ev1', '!r:x', 'todo', 1, $1, 'a todo')",
            from.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();

        MergeService::merge(&pool, &from, &into).await.unwrap();

        let share_owner: uuid::Uuid =
            sqlx::query_scalar!("SELECT owner_id FROM finance_shares WHERE token = 'tok123'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(share_owner, *into.as_uuid());

        let target_owner: uuid::Uuid =
            sqlx::query_scalar!("SELECT owner_id FROM bot_message_targets WHERE event_id = 'ev1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(target_owner, *into.as_uuid());

        for &table in &["finance_shares", "bot_message_targets"] {
            let stranded = repo_merge::count_rows(&pool, table, "owner_id", &from)
                .await
                .unwrap();
            assert_eq!(stranded, 0, "{table} still references the absorbed account");
        }
    }

    #[tokio::test]
    async fn test_a_merge_absorbs_the_audit_ledger() {
        use crate::infrastructure::postgres::repositories::portfolio::audit::record_audit_entry;

        let pool = db().await;
        let from = repo_core::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = repo_core::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();

        record_audit_entry(
            &pool, &into, "deposit", None, None, None, 100000, "USD", 2, None, None, None,
        )
        .await
        .unwrap();
        record_audit_entry(
            &pool, &from, "deposit", None, None, None, 50000, "USD", 2, None, None, None,
        )
        .await
        .unwrap();
        record_audit_entry(
            &pool,
            &from,
            "buy",
            Some("AAPL"),
            Some(1_00000000),
            Some(15000),
            15000,
            "USD",
            2,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        MergeService::merge(&pool, &from, &into)
            .await
            .expect("a merge must succeed and absorb the audit ledger");

        let stranded = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM portfolio_audit_log WHERE owner_id = $1",
            from.as_uuid()
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap_or(0);
        assert_eq!(
            stranded, 0,
            "no ledger row may still reference the absorbed account"
        );

        let arrived = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM portfolio_audit_log WHERE owner_id = $1",
            into.as_uuid()
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap_or(0);
        assert_eq!(arrived, 4, "1 + 2 entries plus the merge marker");
    }

    #[tokio::test]
    async fn test_the_absorbed_ledger_still_verifies() {
        use crate::infrastructure::postgres::repositories::portfolio::audit::{
            record_audit_entry, verify_audit_chain,
        };

        let pool = db().await;
        let from = repo_core::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = repo_core::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();

        record_audit_entry(
            &pool, &into, "deposit", None, None, None, 100000, "USD", 2, None, None, None,
        )
        .await
        .unwrap();
        record_audit_entry(
            &pool, &from, "deposit", None, None, None, 50000, "USD", 2, None, None, None,
        )
        .await
        .unwrap();
        record_audit_entry(
            &pool,
            &from,
            "buy",
            Some("AAPL"),
            Some(1_00000000),
            Some(15000),
            15000,
            "USD",
            2,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        MergeService::merge(&pool, &from, &into).await.unwrap();

        let report = verify_audit_chain(&pool, &into).await.unwrap();
        assert!(report.is_valid, "{}", report.message);
        assert_eq!(report.total_verified, 4);
    }

    #[tokio::test]
    async fn test_the_merge_is_recorded_in_the_ledger() {
        use crate::infrastructure::postgres::repositories::portfolio::audit::record_audit_entry;

        let pool = db().await;
        let from = repo_core::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = repo_core::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();

        record_audit_entry(
            &pool, &from, "deposit", None, None, None, 50000, "USD", 2, None, None, None,
        )
        .await
        .unwrap();

        MergeService::merge(&pool, &from, &into).await.unwrap();

        let markers = sqlx::query!(
            "SELECT id, note FROM portfolio_audit_log WHERE owner_id = $1 AND action = 'merge'",
            into.as_uuid()
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            markers.len(),
            1,
            "exactly one merge marker must be recorded"
        );

        let marker = &markers[0];
        let note = marker.note.as_deref().unwrap_or_default();
        assert!(
            note.contains(&from.to_string()),
            "note must name the source account"
        );
        assert!(
            note.contains('1'),
            "note must say how many entries were absorbed"
        );

        let max_id = sqlx::query_scalar!(
            "SELECT MAX(id) FROM portfolio_audit_log WHERE owner_id = $1",
            into.as_uuid()
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap_or(0);
        assert_eq!(
            marker.id, max_id,
            "the marker must be the last entry in the chain"
        );
    }

    #[tokio::test]
    async fn test_the_immutability_trigger_survives_a_merge() {
        use crate::infrastructure::postgres::repositories::portfolio::audit::record_audit_entry;

        let pool = db().await;
        let from = repo_core::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = repo_core::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();

        record_audit_entry(
            &pool, &from, "deposit", None, None, None, 50000, "USD", 2, None, None, None,
        )
        .await
        .unwrap();

        MergeService::merge(&pool, &from, &into).await.unwrap();

        let result = sqlx::query!(
            "UPDATE portfolio_audit_log SET note = 'tampered' WHERE owner_id = $1",
            into.as_uuid()
        )
        .execute(&pool)
        .await;
        assert!(
            result.is_err(),
            "the immutability trigger must still be active after a merge"
        );
    }

    #[tokio::test]
    async fn test_a_merge_with_no_ledger_leaves_no_marker() {
        use crate::infrastructure::postgres::repositories::portfolio::audit::{
            record_audit_entry, verify_audit_chain,
        };

        let pool = db().await;
        let from = repo_core::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = repo_core::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();

        record_audit_entry(
            &pool, &into, "deposit", None, None, None, 50000, "USD", 2, None, None, None,
        )
        .await
        .unwrap();

        MergeService::merge(&pool, &from, &into).await.unwrap();

        let count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM portfolio_audit_log WHERE owner_id = $1",
            into.as_uuid()
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap_or(0);
        assert_eq!(
            count, 1,
            "no marker row may be appended for an empty ledger"
        );

        let marker_count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM portfolio_audit_log WHERE owner_id = $1 AND action = 'merge'",
            into.as_uuid()
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap_or(0);
        assert_eq!(marker_count, 0);

        let report = verify_audit_chain(&pool, &into).await.unwrap();
        assert!(report.is_valid, "an untouched chain must remain valid");
    }

    #[tokio::test]
    async fn test_a_rolled_back_merge_restores_the_trigger() {
        use crate::infrastructure::postgres::repositories::portfolio::audit::record_audit_entry;

        let pool = db().await;
        let from = repo_core::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = repo_core::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();

        record_audit_entry(
            &pool, &from, "deposit", None, None, None, 50000, "USD", 2, None, None, None,
        )
        .await
        .unwrap();

        let mut tx = pool.begin().await.unwrap();
        MergeService::merge_within(&mut tx, &from, &into)
            .await
            .unwrap();
        tx.rollback().await.unwrap();

        let result = sqlx::query!(
            "UPDATE portfolio_audit_log SET note = 'tampered' WHERE owner_id = $1",
            from.as_uuid()
        )
        .execute(&pool)
        .await;
        assert!(
            result.is_err(),
            "the trigger must still be active after a rolled-back merge"
        );

        let owner: uuid::Uuid = sqlx::query_scalar!(
            r#"SELECT owner_id AS "owner_id: uuid::Uuid" FROM portfolio_audit_log WHERE action = 'deposit'"#
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            owner,
            from.uuid(),
            "the ledger must still belong to the source account"
        );
    }

    #[tokio::test]
    async fn test_cached_rooms_follow_a_merge_and_duplicates_are_dropped() {
        let pool = db().await;
        let from = repo_core::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = repo_core::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();

        sqlx::query!("INSERT INTO account_rooms (account_id, room_id, name) VALUES ($1, '!shared:x', 'Shared')", from.as_uuid()).execute(&pool).await.unwrap();
        sqlx::query!("INSERT INTO account_rooms (account_id, room_id, name) VALUES ($1, '!shared:x', 'Shared')", into.as_uuid()).execute(&pool).await.unwrap();
        sqlx::query!("INSERT INTO account_rooms (account_id, room_id, name) VALUES ($1, '!only-from:x', 'Only from')", from.as_uuid()).execute(&pool).await.unwrap();

        MergeService::merge(&pool, &from, &into).await.unwrap();

        let shared_count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM account_rooms WHERE account_id = $1 AND room_id = '!shared:x'",
            into.as_uuid()
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap_or(0);
        assert_eq!(shared_count, 1, "the shared room must appear exactly once");

        let moved_count = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM account_rooms WHERE account_id = $1 AND room_id = '!only-from:x'",
            into.as_uuid()
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap_or(0);
        assert_eq!(moved_count, 1, "the source-only room must follow");

        let leftover = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM account_rooms WHERE account_id = $1",
            from.as_uuid()
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap_or(0);
        assert_eq!(
            leftover, 0,
            "no row may still reference the absorbed account"
        );
    }

    #[tokio::test]
    async fn test_an_mcp_token_does_not_survive_a_merge() {
        let pool = db().await;
        let from = repo_core::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = repo_core::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();

        sqlx::query!(
            "INSERT INTO mcp_tokens (id, account_id, name, token_hash, prefix) \
             VALUES ('tok1', $1, 'my token', 'hash123', 'mcp_')",
            from.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();

        MergeService::merge(&pool, &from, &into).await.unwrap();

        let remaining = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM mcp_tokens WHERE account_id = $1",
            from.as_uuid()
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap_or(0);
        assert_eq!(remaining, 0, "a credential must not follow a merge");

        let survivor_has_it = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM mcp_tokens WHERE account_id = $1",
            into.as_uuid()
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap_or(0);
        assert_eq!(
            survivor_has_it, 0,
            "it must not appear on the survivor either"
        );
    }

    #[tokio::test]
    async fn test_an_incomplete_merge_leaves_nothing_behind() {
        let pool = db().await;
        let from = repo_core::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = repo_core::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();
        add_todo(&pool, &from, "should not move").await;

        let mut tx = pool.begin().await.unwrap();
        MergeService::merge_within(&mut tx, &from, &into)
            .await
            .unwrap();
        tx.rollback().await.unwrap();

        assert_eq!(owner_of(&pool, "should not move").await, from);
        assert!(repo_core::find(&pool, &from).await.unwrap().is_some());
    }
}
