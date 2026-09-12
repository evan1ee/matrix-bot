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
//! No explicit transaction wraps the rewrite. No repository in this project
//! uses `pool.begin()` — `due_reminder::sync` already does multi-step
//! coordinated writes without one — so this follows the existing precedent
//! rather than introducing a new one. The consequence is stated plainly in
//! [`MergeOutcome`]: an interrupted merge leaves some tables moved and some
//! not, and re-running it finishes the job, because every statement is
//! "re-point rows that still say `from`" and is therefore idempotent.

use sqlx::SqlitePool;

use crate::accounts::OWNED_TABLES;

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
            .execute(pool)
            .await?
            .rows_affected() as i64;

            moved.set(table, updated);

            let leftover: i64 =
                sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table} WHERE owner_id = ?"))
                    .bind(from)
                    .fetch_one(pool)
                    .await
                    .unwrap_or(0);

            if leftover > 0 {
                // Rows the destination already had an equivalent of. Removed so
                // the source account can be deleted without stranding them.
                sqlx::query(&format!("DELETE FROM {table} WHERE owner_id = ?"))
                    .bind(from)
                    .execute(pool)
                    .await?;
                discarded += leftover;
            }
        }

        // Anything else pointing at the absorbed account follows it, so nothing
        // is left referring to an id that is about to disappear.
        for (table, column) in [
            ("matrix_links", "account_id"),
            ("webauthn_credentials", "account_id"),
            ("security_events", "account_id"),
        ] {
            sqlx::query(&format!(
                "UPDATE {table} SET {column} = ? WHERE {column} = ?"
            ))
            .bind(into)
            .bind(from)
            .execute(pool)
            .await?;
        }

        // Sessions and recovery material do *not* follow. A session belongs to
        // the credential that opened it, and carrying one across a merge would
        // silently hand the absorbed account's browser a live session on the
        // surviving account.
        sqlx::query("UPDATE sessions SET revoked_at = CURRENT_TIMESTAMP WHERE account_id = ?")
            .bind(from)
            .execute(pool)
            .await?;
        sqlx::query("DELETE FROM recovery_codes WHERE account_id = ?")
            .bind(from)
            .execute(pool)
            .await?;
        sqlx::query("DELETE FROM recovery_requests WHERE account_id = ?")
            .bind(from)
            .execute(pool)
            .await?;

        sqlx::query("DELETE FROM accounts WHERE id = ?")
            .bind(from)
            .execute(pool)
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
        run_migrations(&pool).await.unwrap();
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

        let preview = MergeRepository::preview(&pool, &from).await.unwrap();
        assert_eq!(preview.total(), 20, "one row per owned table");

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
}
