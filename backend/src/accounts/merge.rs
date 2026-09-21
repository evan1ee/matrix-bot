//! Absorbing one account into another.
//!
//! This runs when someone registers a passkey and then links a Matrix account
//! that already has history — the ordinary case for anyone who used the bot
//! from Matrix before they ever opened the web UI. Their months of todos and
//! transactions belong to an unclaimed account, and linking has to bring them
//! along or the user experiences it as data loss.
//!
//! Both accounts may hold data. There is one code path rather than a cheap
//! "adopt in place" for the empty case plus a real merge for the rest, because
//! the second path has to exist anyway and two paths mean two things to get
//! wrong.
//!
//! The rewrite runs inside one transaction. It previously did not, on the
//! grounds that no repository here used `pool.begin()` — which stopped being
//! true once `modules::portfolio::trade` and `db::rooms` adopted one. A merge
//! moves rows across twenty-odd tables and then deletes the account they used
//! to belong to; a half-applied one is exactly what a transaction is for.
//!
//! `portfolio_audit_log` is the one `owner_id` table the generic loop cannot
//! move: it is immutable by trigger and its hashes are owner-bound. It is
//! handled by `modules::portfolio::audit::absorb_ledger` instead. That is a
//! deliberate reach into a feature module — the rule about what a ledger means
//! across a merge belongs with the ledger, not here.

use sqlx::SqlitePool;

use crate::accounts::{ACCOUNT_ID_DISCARDED, ACCOUNT_ID_FOLLOWS, OWNED_TABLES};

/// How many rows each table would contribute to a merge.
///
/// Shown to the user before they confirm, because "this will move 47 todos and
/// 132 transactions into your account" is a decision someone should get to
/// make with the numbers in front of them.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
pub struct MergePreview {
    pub todos: i64,
    pub reminders: i64,
    pub transactions: i64,
    pub recurring_bills: i64,
    pub portfolio_holdings: i64,
    pub portfolio_cash_balances: i64,
    pub finance_categories: i64,
    pub user_settings: i64,
    pub bookmarks: i64,
    pub habits: i64,
    pub habit_logs: i64,
    pub life_day_logs: i64,
    pub life_profiles: i64,
    pub footprints: i64,
    pub ai_conversations: i64,
    pub webhooks: i64,
    pub other: i64,
}

impl MergePreview {
    pub fn total(&self) -> i64 {
        self.todos
            + self.reminders
            + self.transactions
            + self.recurring_bills
            + self.portfolio_holdings
            + self.portfolio_cash_balances
            + self.finance_categories
            + self.user_settings
            + self.bookmarks
            + self.habits
            + self.habit_logs
            + self.life_day_logs
            + self.life_profiles
            + self.footprints
            + self.ai_conversations
            + self.webhooks
            + self.other
    }

    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }

    fn set(&mut self, table: &str, count: i64) {
        match table {
            "todos" => self.todos = count,
            "reminders" => self.reminders = count,
            "transactions" => self.transactions = count,
            "recurring_bills" => self.recurring_bills = count,
            "portfolio_holdings" => self.portfolio_holdings = count,
            "portfolio_cash_balances" => self.portfolio_cash_balances = count,
            "finance_categories" => self.finance_categories = count,
            "user_settings" => self.user_settings = count,
            "bookmarks" => self.bookmarks = count,
            "habits" => self.habits = count,
            "habit_logs" => self.habit_logs = count,
            "life_day_logs" => self.life_day_logs = count,
            "life_profiles" => self.life_profiles = count,
            "footprints" => self.footprints = count,
            "ai_conversations" => self.ai_conversations = count,
            "webhooks" => self.webhooks = count,
            _ => self.other += count,
        }
    }
}

/// What a completed merge moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeOutcome {
    pub moved: MergePreview,
    /// Rows that could not move because the destination already had the
    /// equivalent row — a settings row, or a category of the same name.
    ///
    /// These are dropped rather than merged: there is no sensible way to
    /// combine two timezone preferences, and the account being kept is the one
    /// whose choices win.
    pub discarded: i64,
}

pub struct MergeRepository;

impl MergeRepository {
    /// Counts what a merge from `from` into `into` would move.
    pub async fn preview(pool: &SqlitePool, from: &str) -> Result<MergePreview, sqlx::Error> {
        let mut preview = MergePreview::default();

        for table in OWNED_TABLES {
            let count: i64 =
                sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table} WHERE owner_id = ?"))
                    .bind(from)
                    .fetch_one(pool)
                    .await
                    .unwrap_or(0);

            preview.set(table, count);
        }

        // Counted even though the generic loop cannot move it, so the number a
        // user is shown before confirming matches what the merge reports after.
        for table in crate::accounts::OWNER_ID_MERGED_SEPARATELY {
            let count: i64 =
                sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table} WHERE owner_id = ?"))
                    .bind(from)
                    .fetch_one(pool)
                    .await
                    .unwrap_or(0);

            preview.set(table, count);
        }

        Ok(preview)
    }

    /// Moves everything `from` owns to `into`, then deletes `from`.
    ///
    /// Refuses to merge an account into itself, which would otherwise delete a
    /// live account after "moving" all its rows to where they already were.
    pub async fn merge(
        pool: &SqlitePool,
        from: &str,
        into: &str,
    ) -> Result<MergeOutcome, sqlx::Error> {
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
    pub(crate) async fn merge_within(
        conn: &mut sqlx::SqliteConnection,
        from: &str,
        into: &str,
    ) -> Result<MergeOutcome, sqlx::Error> {
        let mut moved = MergePreview::default();
        let mut discarded = 0i64;

        for table in OWNED_TABLES {
            // `OR IGNORE` because three of these tables have a unique index on
            // `(owner_id, …)`: settings are one row per owner, categories are
            // unique by name, holdings by symbol, cash by currency. Where both
            // accounts have the equivalent row the destination's is kept, and
            // the source's is dropped rather than colliding and aborting the
            // whole merge.
            let updated = sqlx::query(&format!(
                "UPDATE OR IGNORE {table} SET owner_id = ? WHERE owner_id = ?"
            ))
            .bind(into)
            .bind(from)
            .execute(&mut *conn)
            .await?
            .rows_affected() as i64;

            moved.set(table, updated);

            let leftover: i64 =
                sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table} WHERE owner_id = ?"))
                    .bind(from)
                    .fetch_one(&mut *conn)
                    .await
                    .unwrap_or(0);

            if leftover > 0 {
                // Rows the destination already had an equivalent of. Removed so
                // the source account can be deleted without stranding them.
                sqlx::query(&format!("DELETE FROM {table} WHERE owner_id = ?"))
                    .bind(from)
                    .execute(&mut *conn)
                    .await?;
                discarded += leftover;
            }
        }

        // The audit ledger cannot go through the loop above: it is immutable by
        // trigger and every hash is bound to the owner, so it needs the routine
        // that knows how to rebuild it.
        let ledger_rows =
            crate::modules::portfolio::audit::absorb_ledger(&mut *conn, from, into).await?;
        moved.set("portfolio_audit_log", ledger_rows);

        // Anything else pointing at the absorbed account follows it, so nothing
        // is left referring to an id that is about to disappear.
        //
        // `OR IGNORE` for the same reason as above: `account_rooms` is keyed by
        // `(account_id, room_id)`, so a room both accounts had cached would
        // otherwise collide and abort the whole merge.
        for table in ACCOUNT_ID_FOLLOWS {
            sqlx::query(&format!(
                "UPDATE OR IGNORE {table} SET account_id = ? WHERE account_id = ?"
            ))
            .bind(into)
            .bind(from)
            .execute(&mut *conn)
            .await?;

            let leftover: i64 = sqlx::query_scalar(&format!(
                "SELECT COUNT(*) FROM {table} WHERE account_id = ?"
            ))
            .bind(from)
            .fetch_one(&mut *conn)
            .await
            .unwrap_or(0);

            if leftover > 0 {
                sqlx::query(&format!("DELETE FROM {table} WHERE account_id = ?"))
                    .bind(from)
                    .execute(&mut *conn)
                    .await?;
                discarded += leftover;
            }
        }

        // Credentials and ceremonies do *not* follow. A session belongs to the
        // credential that opened it, and carrying one across a merge would
        // silently hand the absorbed account's browser a live session on the
        // surviving account. The same argument covers MCP tokens, enrolment
        // tokens and link challenges.
        for table in ACCOUNT_ID_DISCARDED {
            let statement = if *table == "sessions" {
                // Revoked rather than deleted: the row is the record that the
                // session existed, which the security log refers back to.
                "UPDATE sessions SET revoked_at = CURRENT_TIMESTAMP WHERE account_id = ?"
                    .to_string()
            } else {
                format!("DELETE FROM {table} WHERE account_id = ?")
            };

            sqlx::query(&statement)
                .bind(from)
                .execute(&mut *conn)
                .await?;
        }

        sqlx::query("DELETE FROM accounts WHERE id = ?")
            .bind(from)
            .execute(&mut *conn)
            .await?;

        Ok(MergeOutcome { moved, discarded })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{AccountRepository, STATUS_ACTIVE, STATUS_UNCLAIMED};
    use crate::db::schema::run_migrations;

    async fn db() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&crate::db::DbPool::Sqlite(pool.clone()))
            .await
            .unwrap();
        pool
    }

    async fn add_todo(pool: &SqlitePool, owner: &str, text: &str) {
        sqlx::query(
            "INSERT INTO todos (room_id, owner_id, sender_name, text, priority, completed) \
             VALUES ('!r:x', ?, 'someone', ?, 'medium', 0)",
        )
        .bind(owner)
        .bind(text)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn owner_of(pool: &SqlitePool, text: &str) -> String {
        sqlx::query_scalar("SELECT owner_id FROM todos WHERE text = ?")
            .bind(text)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_preview_counts_what_would_move() {
        let pool = db().await;
        let from = AccountRepository::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();

        add_todo(&pool, &from, "one").await;
        add_todo(&pool, &from, "two").await;
        sqlx::query(
            "INSERT INTO user_settings (owner_id, timezone, currency) VALUES (?, 'UTC', 'USD')",
        )
        .bind(&from)
        .execute(&pool)
        .await
        .unwrap();

        let preview = MergeRepository::preview(&pool, &from).await.unwrap();
        assert_eq!(preview.todos, 2);
        assert_eq!(preview.user_settings, 1);
        assert_eq!(preview.total(), 3);
        assert!(!preview.is_empty());
    }

    /// The headline case: months of Matrix history following the user onto the
    /// account they just made a passkey for.
    #[tokio::test]
    async fn test_merge_moves_the_rows_and_removes_the_source() {
        let pool = db().await;
        let from = AccountRepository::create(&pool, "matrix", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = AccountRepository::create(&pool, "web", STATUS_ACTIVE)
            .await
            .unwrap();

        add_todo(&pool, &from, "from matrix").await;
        add_todo(&pool, &into, "from the web").await;

        let outcome = MergeRepository::merge(&pool, &from, &into).await.unwrap();
        assert_eq!(outcome.moved.todos, 1);
        assert_eq!(outcome.discarded, 0);

        // Both rows now belong to the surviving account.
        assert_eq!(owner_of(&pool, "from matrix").await, into);
        assert_eq!(owner_of(&pool, "from the web").await, into);

        assert!(AccountRepository::find(&pool, &from)
            .await
            .unwrap()
            .is_none());
        assert!(AccountRepository::find(&pool, &into)
            .await
            .unwrap()
            .is_some());
    }

    /// The linked MXID has to follow, or the user merges their history in and
    /// then cannot reach the account from Matrix any more.
    #[tokio::test]
    async fn test_merge_carries_the_matrix_link_over() {
        let pool = db().await;
        let from = AccountRepository::resolve_or_provision(&pool, "@evan:mbm.one")
            .await
            .unwrap();
        let into = AccountRepository::create(&pool, "web", STATUS_ACTIVE)
            .await
            .unwrap();

        MergeRepository::merge(&pool, &from, &into).await.unwrap();

        assert_eq!(
            AccountRepository::find_by_mxid(&pool, "@evan:mbm.one")
                .await
                .unwrap(),
            Some(into.clone())
        );
        assert_eq!(
            AccountRepository::linked_mxids(&pool, &into).await.unwrap(),
            vec!["@evan:mbm.one".to_string()]
        );
    }

    /// Where both accounts have a row the destination already has an
    /// equivalent of, the destination's wins and the source's is dropped —
    /// there is no way to average two timezones.
    #[tokio::test]
    async fn test_conflicting_singleton_rows_keep_the_destinations() {
        let pool = db().await;
        let from = AccountRepository::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = AccountRepository::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();

        for (owner, tz) in [(&from, "Asia/Tokyo"), (&into, "Europe/Berlin")] {
            sqlx::query(
                "INSERT INTO user_settings (owner_id, timezone, currency) VALUES (?, ?, 'USD')",
            )
            .bind(owner)
            .bind(tz)
            .execute(&pool)
            .await
            .unwrap();
        }

        let outcome = MergeRepository::merge(&pool, &from, &into).await.unwrap();
        assert_eq!(outcome.discarded, 1, "the source's settings row is dropped");

        let tz: String =
            sqlx::query_scalar("SELECT timezone FROM user_settings WHERE owner_id = ?")
                .bind(&into)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(tz, "Europe/Berlin", "the surviving account's choice wins");

        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_settings")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 1, "the absorbed row must not be stranded");
    }

    /// A merged-away account's sessions must not survive: the browser holding
    /// one would otherwise find itself signed in to the surviving account.
    #[tokio::test]
    async fn test_the_absorbed_accounts_sessions_are_revoked() {
        use crate::accounts::SessionRepository;

        let pool = db().await;
        let from = AccountRepository::create(&pool, "old", STATUS_ACTIVE)
            .await
            .unwrap();
        let into = AccountRepository::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();

        let old_session = SessionRepository::create(&pool, &from, None, None, None)
            .await
            .unwrap();
        let kept_session = SessionRepository::create(&pool, &into, None, None, None)
            .await
            .unwrap();

        MergeRepository::merge(&pool, &from, &into).await.unwrap();

        assert!(
            SessionRepository::resolve(&pool, &old_session)
                .await
                .unwrap()
                .is_none(),
            "the absorbed account's session must not become a session on the survivor"
        );
        assert!(SessionRepository::resolve(&pool, &kept_session)
            .await
            .unwrap()
            .is_some());
    }

    /// Merging an account into itself would delete it after moving its rows to
    /// where they already were.
    #[tokio::test]
    async fn test_self_merge_is_refused() {
        let pool = db().await;
        let account = AccountRepository::create(&pool, "solo", STATUS_ACTIVE)
            .await
            .unwrap();
        add_todo(&pool, &account, "keep me").await;

        let outcome = MergeRepository::merge(&pool, &account, &account)
            .await
            .unwrap();
        assert_eq!(outcome.moved.total(), 0);

        assert!(AccountRepository::find(&pool, &account)
            .await
            .unwrap()
            .is_some());
        assert_eq!(owner_of(&pool, "keep me").await, account);
    }

    /// Every statement is "re-point rows that still say `from`", so finishing
    /// an interrupted merge is just running it again.
    #[tokio::test]
    async fn test_merging_twice_is_harmless() {
        let pool = db().await;
        let from = AccountRepository::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = AccountRepository::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();
        add_todo(&pool, &from, "only me").await;

        MergeRepository::merge(&pool, &from, &into).await.unwrap();
        let second = MergeRepository::merge(&pool, &from, &into).await.unwrap();

        assert_eq!(second.moved.total(), 0);
        assert_eq!(owner_of(&pool, "only me").await, into);
    }

    /// Every ownership-carrying table must actually be covered. A table missing
    /// from `OWNED_TABLES` would leave rows behind pointing at a deleted
    /// account — invisible to their owner and impossible to reach.
    #[tokio::test]
    async fn test_no_owned_table_is_left_behind() {
        let pool = db().await;
        let from = AccountRepository::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = AccountRepository::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();

        // Put one row in every owned table.
        add_todo(&pool, &from, "todo").await;
        sqlx::query("INSERT INTO reminders (room_id, owner_id, sender_name, message, trigger_at) VALUES ('!r:x', ?, 'a', 'm', CURRENT_TIMESTAMP)")
            .bind(&from).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO transactions (room_id, owner_id, sender_name, direction, amount_minor, currency, occurred_at) VALUES ('web', ?, 'a', 'expense', 100, 'USD', CURRENT_TIMESTAMP)")
            .bind(&from).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO recurring_bills (room_id, owner_id, sender_name, name, category, amount_minor, currency, next_due_at) VALUES ('web', ?, 'a', 'rent', 'home', 100, 'USD', CURRENT_TIMESTAMP)")
            .bind(&from).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO portfolio_holdings (owner_id, symbol, quantity_minor, avg_cost_minor, currency) VALUES (?, 'AAPL', 100000000, 15000, 'USD')")
            .bind(&from).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO portfolio_cash_balances (owner_id, currency, balance_minor) VALUES (?, 'USD', 5000)")
            .bind(&from).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO portfolio_daily_snapshots (owner_id, snapshot_date, currency, holdings_value_minor, cash_minor, total_net_worth_minor) VALUES (?, '2026-09-06', 'USD', 100, 100, 200)")
            .bind(&from).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO portfolio_price_alerts (owner_id, symbol, target_price_minor, currency, condition) VALUES (?, 'AAPL', 20000, 'USD', 'ABOVE')")
            .bind(&from).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO finance_categories (owner_id, name) VALUES (?, 'food')")
            .bind(&from)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO user_settings (owner_id, timezone, currency) VALUES (?, 'UTC', 'USD')",
        )
        .bind(&from)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO bookmarks (owner_id, url, title, domain) VALUES (?, 'https://example.com', 'Example', 'example.com')")
            .bind(&from).execute(&pool).await.unwrap();
        let habit_id: i64 = sqlx::query_scalar("INSERT INTO habits (owner_id, name, frequency) VALUES (?, 'Exercise', 'daily') RETURNING id")
            .bind(&from).fetch_one(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO habit_logs (habit_id, owner_id, log_date) VALUES (?, ?, '2026-09-06')",
        )
        .bind(habit_id)
        .bind(&from)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO life_day_logs (owner_id, log_date, mood) VALUES (?, '2026-09-06', 'happy')")
            .bind(&from).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO life_profiles (owner_id, birth_date, expected_lifespan_years) VALUES (?, '1990-01-01', 90)")
            .bind(&from).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO footprints (owner_id, title, latitude, longitude, arrival_date) VALUES (?, 'Paris', 48.8566, 2.3522, '2026-09-06')")
            .bind(&from).execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO ai_conversations (id, owner_id, room_id) VALUES ('conv1', ?, '!r:x')",
        )
        .bind(&from)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO ai_user_configs (owner_id, model_name) VALUES (?, 'gemini')")
            .bind(&from)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO webhooks (id, owner_id, name, token_hash, token_prefix, default_room_id) VALUES ('wh1', ?, 'hook', 'thash', 'pref', '!r:x')")
            .bind(&from).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO webhook_outbox (id, webhook_id, owner_id, room_id, html_body, plain_body, next_attempt_at) VALUES ('out1', 'wh1', ?, '!r:x', 'html', 'plain', CURRENT_TIMESTAMP)")
            .bind(&from).execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO finance_shares (owner_id, token, is_enabled) VALUES (?, 'tok1', 1)",
        )
        .bind(&from)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO bot_message_targets (event_id, room_id, entity_type, entity_id, owner_id, title) \
             VALUES ('ev1', '!r:x', 'todo', 1, ?, 'a todo')",
        )
        .bind(&from)
        .execute(&pool)
        .await
        .unwrap();

        let preview = MergeRepository::preview(&pool, &from).await.unwrap();
        assert_eq!(preview.total(), 22, "one row per owned table");

        MergeRepository::merge(&pool, &from, &into).await.unwrap();

        for table in OWNED_TABLES {
            let stranded: i64 =
                sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table} WHERE owner_id = ?"))
                    .bind(&from)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(
                stranded, 0,
                "{table} still has rows owned by the absorbed account"
            );

            let arrived: i64 =
                sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table} WHERE owner_id = ?"))
                    .bind(&into)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(arrived, 1, "{table} did not receive its row");
        }
    }

    /// These two tables were missing from `OWNED_TABLES` until this change —
    /// without it, a finance share link and a reaction target were stranded on
    /// a deleted account.
    #[tokio::test]
    async fn test_a_merge_moves_the_finance_share_and_reaction_targets() {
        let pool = db().await;
        let from = AccountRepository::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = AccountRepository::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();

        sqlx::query(
            "INSERT INTO finance_shares (owner_id, token, is_enabled) VALUES (?, 'tok123', 1)",
        )
        .bind(&from)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO bot_message_targets (event_id, room_id, entity_type, entity_id, owner_id, title) \
             VALUES ('ev1', '!r:x', 'todo', 1, ?, 'a todo')",
        )
        .bind(&from)
        .execute(&pool)
        .await
        .unwrap();

        MergeRepository::merge(&pool, &from, &into).await.unwrap();

        let share_owner: String =
            sqlx::query_scalar("SELECT owner_id FROM finance_shares WHERE token = 'tok123'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(share_owner, into);

        let target_owner: String =
            sqlx::query_scalar("SELECT owner_id FROM bot_message_targets WHERE event_id = 'ev1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(target_owner, into);

        for table in ["finance_shares", "bot_message_targets"] {
            let stranded: i64 =
                sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table} WHERE owner_id = ?"))
                    .bind(&from)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(stranded, 0, "{table} still references the absorbed account");
        }
    }

    /// The headline case: the audit ledger cannot go through the generic
    /// `owner_id` loop (immutable by trigger, hashes bound to the owner), so a
    /// merge routes it through `absorb_ledger` instead. This pins that both
    /// accounts' entries end up on the survivor, plus one marker row recording
    /// the absorption itself.
    #[tokio::test]
    async fn test_a_merge_absorbs_the_audit_ledger() {
        use crate::modules::portfolio::audit::record_audit_entry;

        let pool = db().await;
        let from = AccountRepository::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = AccountRepository::create(&pool, "new", STATUS_ACTIVE)
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

        MergeRepository::merge(&pool, &from, &into)
            .await
            .expect("a merge must succeed and absorb the audit ledger");

        let stranded: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM portfolio_audit_log WHERE owner_id = ?")
                .bind(&from)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            stranded, 0,
            "no ledger row may still reference the absorbed account"
        );

        let arrived: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM portfolio_audit_log WHERE owner_id = ?")
                .bind(&into)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(arrived, 4, "1 + 2 entries plus the merge marker");
    }

    /// This is the test the whole change exists for: the rebuilt chain must
    /// still verify from genesis to head after the ledgers of two different
    /// accounts have been spliced together.
    #[tokio::test]
    async fn test_the_absorbed_ledger_still_verifies() {
        use crate::modules::portfolio::audit::{record_audit_entry, verify_audit_chain};

        let pool = db().await;
        let from = AccountRepository::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = AccountRepository::create(&pool, "new", STATUS_ACTIVE)
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

        MergeRepository::merge(&pool, &from, &into).await.unwrap();

        let report = verify_audit_chain(&pool, &into).await.unwrap();
        assert!(report.is_valid, "{}", report.message);
        assert_eq!(report.total_verified, 4);
    }

    /// The absorption itself must be recorded in the ledger rather than
    /// happening silently — an auditor reading the chain should be able to see
    /// that a merge occurred and where the entries came from.
    #[tokio::test]
    async fn test_the_merge_is_recorded_in_the_ledger() {
        use crate::modules::portfolio::audit::record_audit_entry;

        let pool = db().await;
        let from = AccountRepository::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = AccountRepository::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();

        record_audit_entry(
            &pool, &from, "deposit", None, None, None, 50000, "USD", 2, None, None, None,
        )
        .await
        .unwrap();

        MergeRepository::merge(&pool, &from, &into).await.unwrap();

        let markers: Vec<(i64, Option<String>)> = sqlx::query_as(
            "SELECT id, note FROM portfolio_audit_log WHERE owner_id = ? AND action = 'merge'",
        )
        .bind(&into)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            markers.len(),
            1,
            "exactly one merge marker must be recorded"
        );

        let (marker_id, note) = &markers[0];
        let note = note.as_deref().unwrap_or_default();
        assert!(note.contains(&from), "note must name the source account");
        assert!(
            note.contains('1'),
            "note must say how many entries were absorbed"
        );

        let max_id: i64 =
            sqlx::query_scalar("SELECT MAX(id) FROM portfolio_audit_log WHERE owner_id = ?")
                .bind(&into)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            *marker_id, max_id,
            "the marker must be the last entry in the chain"
        );
    }

    /// Without this, a bug that skipped recreating the trigger would leave the
    /// ledger permanently writable after the first merge.
    #[tokio::test]
    async fn test_the_immutability_trigger_survives_a_merge() {
        use crate::modules::portfolio::audit::record_audit_entry;

        let pool = db().await;
        let from = AccountRepository::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = AccountRepository::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();

        record_audit_entry(
            &pool, &from, "deposit", None, None, None, 50000, "USD", 2, None, None, None,
        )
        .await
        .unwrap();

        MergeRepository::merge(&pool, &from, &into).await.unwrap();

        let result =
            sqlx::query("UPDATE portfolio_audit_log SET note = 'tampered' WHERE owner_id = ?")
                .bind(&into)
                .execute(&pool)
                .await;
        assert!(
            result.is_err(),
            "the immutability trigger must still be active after a merge"
        );
    }

    /// An account with no audit history must not cause a merge to append noise
    /// or rewrite a chain that never needed rebuilding.
    #[tokio::test]
    async fn test_a_merge_with_no_ledger_leaves_no_marker() {
        use crate::modules::portfolio::audit::{record_audit_entry, verify_audit_chain};

        let pool = db().await;
        let from = AccountRepository::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = AccountRepository::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();

        record_audit_entry(
            &pool, &into, "deposit", None, None, None, 50000, "USD", 2, None, None, None,
        )
        .await
        .unwrap();

        MergeRepository::merge(&pool, &from, &into).await.unwrap();

        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM portfolio_audit_log WHERE owner_id = ?")
                .bind(&into)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            count, 1,
            "no marker row may be appended for an empty ledger"
        );

        let marker_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM portfolio_audit_log WHERE owner_id = ? AND action = 'merge'",
        )
        .bind(&into)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(marker_count, 0);

        let report = verify_audit_chain(&pool, &into).await.unwrap();
        assert!(report.is_valid, "an untouched chain must remain valid");
    }

    /// Proves the transactional-DDL assumption `absorb_ledger` relies on: a
    /// rolled-back merge must leave the trigger intact, because SQLite undoes
    /// `DROP TRIGGER` along with everything else in the transaction.
    #[tokio::test]
    async fn test_a_rolled_back_merge_restores_the_trigger() {
        use crate::modules::portfolio::audit::record_audit_entry;

        let pool = db().await;
        let from = AccountRepository::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = AccountRepository::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();

        record_audit_entry(
            &pool, &from, "deposit", None, None, None, 50000, "USD", 2, None, None, None,
        )
        .await
        .unwrap();

        let mut tx = pool.begin().await.unwrap();
        MergeRepository::merge_within(&mut tx, &from, &into)
            .await
            .unwrap();
        tx.rollback().await.unwrap();

        let result =
            sqlx::query("UPDATE portfolio_audit_log SET note = 'tampered' WHERE owner_id = ?")
                .bind(&from)
                .execute(&pool)
                .await;
        assert!(
            result.is_err(),
            "the trigger must still be active after a rolled-back merge"
        );

        let owner: String =
            sqlx::query_scalar("SELECT owner_id FROM portfolio_audit_log WHERE action = 'deposit'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            owner, from,
            "the ledger must still belong to the source account"
        );
    }

    /// `account_rooms` is keyed by `(account_id, room_id)`, so a room cached by
    /// both accounts must not abort the merge — the destination's copy wins and
    /// the duplicate is dropped.
    #[tokio::test]
    async fn test_cached_rooms_follow_a_merge_and_duplicates_are_dropped() {
        let pool = db().await;
        let from = AccountRepository::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = AccountRepository::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();

        sqlx::query(
            "INSERT INTO account_rooms (account_id, room_id, name) VALUES (?, '!shared:x', 'Shared')",
        )
        .bind(&from)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO account_rooms (account_id, room_id, name) VALUES (?, '!shared:x', 'Shared')",
        )
        .bind(&into)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO account_rooms (account_id, room_id, name) VALUES (?, '!only-from:x', 'Only from')",
        )
        .bind(&from)
        .execute(&pool)
        .await
        .unwrap();

        MergeRepository::merge(&pool, &from, &into).await.unwrap();

        let shared_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM account_rooms WHERE account_id = ? AND room_id = '!shared:x'",
        )
        .bind(&into)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(shared_count, 1, "the shared room must appear exactly once");

        let moved_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM account_rooms WHERE account_id = ? AND room_id = '!only-from:x'",
        )
        .bind(&into)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(moved_count, 1, "the source-only room must follow");

        let leftover: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM account_rooms WHERE account_id = ?")
                .bind(&from)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            leftover, 0,
            "no row may still reference the absorbed account"
        );
    }

    /// A credential must not survive a merge: it would hand whoever held the
    /// absorbed account a live way into the surviving one.
    #[tokio::test]
    async fn test_an_mcp_token_does_not_survive_a_merge() {
        let pool = db().await;
        let from = AccountRepository::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = AccountRepository::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();

        sqlx::query(
            "INSERT INTO mcp_tokens (id, account_id, name, token_hash, prefix) \
             VALUES ('tok1', ?, 'my token', 'hash123', 'mcp_')",
        )
        .bind(&from)
        .execute(&pool)
        .await
        .unwrap();

        MergeRepository::merge(&pool, &from, &into).await.unwrap();

        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM mcp_tokens WHERE account_id = ?")
                .bind(&from)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(remaining, 0, "a credential must not follow a merge");

        let survivor_has_it: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM mcp_tokens WHERE account_id = ?")
                .bind(&into)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            survivor_has_it, 0,
            "it must not appear on the survivor either"
        );
    }

    /// This is the test the transaction exists for: if a merge cannot finish,
    /// nothing it touched should be visible afterward.
    #[tokio::test]
    async fn test_an_incomplete_merge_leaves_nothing_behind() {
        let pool = db().await;
        let from = AccountRepository::create(&pool, "old", STATUS_UNCLAIMED)
            .await
            .unwrap();
        let into = AccountRepository::create(&pool, "new", STATUS_ACTIVE)
            .await
            .unwrap();
        add_todo(&pool, &from, "should not move").await;

        let mut tx = pool.begin().await.unwrap();
        MergeRepository::merge_within(&mut tx, &from, &into)
            .await
            .unwrap();
        tx.rollback().await.unwrap();

        assert_eq!(owner_of(&pool, "should not move").await, from);
        assert!(
            AccountRepository::find(&pool, &from)
                .await
                .unwrap()
                .is_some(),
            "the source account must still exist after a rolled-back merge"
        );
    }
}
