//! PostgreSQL dynamic SQL for account merging behind compile-time allow-lists (Decision D-A).
//!
//! Because a merge iterates `OWNED_TABLES` and rewrites ownership across 20+ tables,
//! this file uses dynamic identifier interpolation behind explicit allow-list guards.
//! Every value is bound as a parameter ($1, $2, ...), and SQL statements are wrapped in
//! `sqlx::AssertSqlSafe` only within this file.

use sqlx::AssertSqlSafe;

use crate::domain::accounts::{
    AccountId, MergePreview, ACCOUNT_ID_DISCARDED, ACCOUNT_ID_FOLLOWS, ACCOUNT_ID_UNTOUCHED,
    OWNED_TABLES, OWNER_ID_MERGED_SEPARATELY,
};

/// Asserts that a table name is in the known allow-list of tables affected by an account merge.
pub fn assert_allowed_table(table: &str) {
    let allowed = OWNED_TABLES
        .iter()
        .chain(OWNER_ID_MERGED_SEPARATELY.iter())
        .chain(ACCOUNT_ID_FOLLOWS.iter())
        .chain(ACCOUNT_ID_DISCARDED.iter())
        .chain(ACCOUNT_ID_UNTOUCHED.iter())
        .any(|&t| t == table);
    assert!(allowed, "table '{table}' is not in the merge allow-list");
}

/// Asserts that a column name is an allowed ownership identifier.
pub fn assert_allowed_column(column: &str) {
    let allowed = matches!(column, "owner_id" | "account_id" | "id");
    assert!(allowed, "column '{column}' is not an allowed merge column");
}

/// The column identifying one row within an owner's set.
///
/// Used only by the per-row fallback in `move_rows`.
pub fn row_key(table: &str) -> &'static str {
    assert_allowed_table(table);
    match table {
        "finance_shares" | "user_settings" | "life_profiles" | "ai_user_configs" => "owner_id",
        "bot_message_targets" => "event_id",
        "account_rooms" => "room_id",
        _ => "id",
    }
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(e) if e.is_unique_violation())
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ReconciledCollisions {
    pub cash_balances: i64,
    pub holdings: i64,
    pub snapshots: i64,
}

/// Reconciles owned rows that would collide on unique constraints when `from` is
/// merged into `into`:
/// 1. Remaps `transactions.category_id` from `from`'s `finance_categories` to
///    `into`'s matching category (`LOWER(name)`) so moved transactions do not
///    lose their category when `from`'s duplicate category row is deleted.
///    Budgets follow in 1b: `from`'s budgets whose scope + currency `into`
///    already has are deleted (`into`'s wins), as are `from`'s own case-variant
///    duplicates that would remap onto one category (lowest id wins); the rest
///    are remapped the same way.
/// 2. Sums colliding `portfolio_cash_balances` (`(owner_id, currency)`).
/// 3. Combines colliding `portfolio_holdings` (`(owner_id, symbol)` with same
///    `currency`) by summing quantities and computing weighted average cost.
/// 4. Sums colliding `portfolio_daily_snapshots` (`(owner_id, snapshot_date, currency)`).
/// 5. Drops `from`'s `briefing_audio` for days `into` already has (`into` wins).
///    The merge holds no storage handle, so the dropped object stays behind,
///    unreachable: at most one small recording per clashing day inside the
///    30-day retention window. Surviving rows keep their key, which still names
///    the old account's prefix; readers trust the owner-scoped row, not the key.
pub async fn reconcile_colliding_owned_rows(
    conn: &mut sqlx::PgConnection,
    from: &AccountId,
    into: &AccountId,
) -> Result<ReconciledCollisions, sqlx::Error> {
    let mut out = ReconciledCollisions::default();

    // 1. Remap transactions.category_id for colliding finance_categories
    sqlx::query(AssertSqlSafe(
        "UPDATE transactions t \
         SET category_id = fc_into.id \
         FROM finance_categories fc_from \
         JOIN finance_categories fc_into \
           ON fc_into.owner_id = $1 AND LOWER(fc_into.name) = LOWER(fc_from.name) \
         WHERE t.owner_id = $2 \
           AND fc_from.owner_id = $2 \
           AND t.category_id = fc_from.id",
    ))
    .bind(into.as_uuid())
    .bind(from.as_uuid())
    .execute(&mut *conn)
    .await?;

    // 1b. Budgets: `into`'s budget wins a scope + currency collision, so drop
    // `from`'s colliding budgets first (overall ones, and category ones whose
    // category matches an `into` category by name); a surviving budget on a
    // duplicate category would otherwise move over still pointing at it and
    // cascade away with that category. Then remap the rest like transactions.
    sqlx::query(AssertSqlSafe(
        "DELETE FROM finance_budgets b \
         WHERE b.owner_id = $2 \
           AND EXISTS ( \
               SELECT 1 FROM finance_budgets o \
               LEFT JOIN finance_categories fc_o ON fc_o.id = o.category_id \
               LEFT JOIN finance_categories fc_b ON fc_b.id = b.category_id \
               WHERE o.owner_id = $1 AND o.currency = b.currency \
                 AND ((o.category_id IS NULL AND b.category_id IS NULL) \
                      OR LOWER(fc_o.name) = LOWER(fc_b.name)))",
    ))
    .bind(into.as_uuid())
    .bind(from.as_uuid())
    .execute(&mut *conn)
    .await?;
    // Category names are unique per exact spelling, so `from` may hold "Food" and
    // "food" with a budget each; both remap to the one `into` category and would
    // violate `idx_finance_budgets_scope`. Keep the lowest id per target + currency.
    sqlx::query(AssertSqlSafe(
        "DELETE FROM finance_budgets b \
         USING finance_categories fc_b \
         WHERE b.owner_id = $2 \
           AND fc_b.id = b.category_id \
           AND EXISTS ( \
               SELECT 1 FROM finance_categories fc_into \
               WHERE fc_into.owner_id = $1 AND LOWER(fc_into.name) = LOWER(fc_b.name)) \
           AND EXISTS ( \
               SELECT 1 FROM finance_budgets b2 \
               JOIN finance_categories fc_b2 ON fc_b2.id = b2.category_id \
               WHERE b2.owner_id = $2 AND b2.currency = b.currency AND b2.id < b.id \
                 AND LOWER(fc_b2.name) = LOWER(fc_b.name))",
    ))
    .bind(into.as_uuid())
    .bind(from.as_uuid())
    .execute(&mut *conn)
    .await?;
    sqlx::query(AssertSqlSafe(
        "UPDATE finance_budgets b \
         SET category_id = fc_into.id \
         FROM finance_categories fc_from \
         JOIN finance_categories fc_into \
           ON fc_into.owner_id = $1 AND LOWER(fc_into.name) = LOWER(fc_from.name) \
         WHERE b.owner_id = $2 \
           AND fc_from.owner_id = $2 \
           AND b.category_id = fc_from.id",
    ))
    .bind(into.as_uuid())
    .bind(from.as_uuid())
    .execute(&mut *conn)
    .await?;

    // 2. Sum colliding portfolio_cash_balances
    let cash_merged = sqlx::query(AssertSqlSafe(
        "UPDATE portfolio_cash_balances into_cb \
         SET balance_minor = into_cb.balance_minor + from_cb.balance_minor, \
             updated_at = CURRENT_TIMESTAMP \
         FROM portfolio_cash_balances from_cb \
         WHERE into_cb.owner_id = $1 \
           AND from_cb.owner_id = $2 \
           AND into_cb.currency = from_cb.currency",
    ))
    .bind(into.as_uuid())
    .bind(from.as_uuid())
    .execute(&mut *conn)
    .await?;
    if cash_merged.rows_affected() > 0 {
        sqlx::query(AssertSqlSafe(
            "DELETE FROM portfolio_cash_balances from_cb \
             USING portfolio_cash_balances into_cb \
             WHERE from_cb.owner_id = $1 \
               AND into_cb.owner_id = $2 \
               AND from_cb.currency = into_cb.currency",
        ))
        .bind(from.as_uuid())
        .bind(into.as_uuid())
        .execute(&mut *conn)
        .await?;
        out.cash_balances = cash_merged.rows_affected() as i64;
    }

    // 3. Combine colliding portfolio_holdings (same symbol and currency)
    let holdings_merged = sqlx::query(AssertSqlSafe(
        "UPDATE portfolio_holdings into_h \
         SET avg_cost_minor = CASE \
                 WHEN (into_h.quantity_minor + from_h.quantity_minor) > 0 THEN \
                     CAST( \
                         (CAST(into_h.quantity_minor AS NUMERIC) * CAST(into_h.avg_cost_minor AS NUMERIC) + \
                          CAST(from_h.quantity_minor AS NUMERIC) * CAST(from_h.avg_cost_minor AS NUMERIC)) \
                         / CAST(into_h.quantity_minor + from_h.quantity_minor AS NUMERIC) \
                         AS BIGINT \
                     ) \
                 ELSE into_h.avg_cost_minor \
             END, \
             quantity_minor = into_h.quantity_minor + from_h.quantity_minor, \
             updated_at = CURRENT_TIMESTAMP \
         FROM portfolio_holdings from_h \
         WHERE into_h.owner_id = $1 \
           AND from_h.owner_id = $2 \
           AND into_h.symbol = from_h.symbol \
           AND into_h.currency = from_h.currency",
    ))
    .bind(into.as_uuid())
    .bind(from.as_uuid())
    .execute(&mut *conn)
    .await?;
    if holdings_merged.rows_affected() > 0 {
        sqlx::query(AssertSqlSafe(
            "DELETE FROM portfolio_holdings from_h \
             USING portfolio_holdings into_h \
             WHERE from_h.owner_id = $1 \
               AND into_h.owner_id = $2 \
               AND from_h.symbol = into_h.symbol \
               AND from_h.currency = into_h.currency",
        ))
        .bind(from.as_uuid())
        .bind(into.as_uuid())
        .execute(&mut *conn)
        .await?;
        out.holdings = holdings_merged.rows_affected() as i64;
    }

    // 4. Sum colliding portfolio_daily_snapshots
    let snapshots_merged = sqlx::query(AssertSqlSafe(
        "UPDATE portfolio_daily_snapshots into_s \
         SET holdings_value_minor = into_s.holdings_value_minor + from_s.holdings_value_minor, \
             cash_minor = into_s.cash_minor + from_s.cash_minor, \
             total_net_worth_minor = into_s.total_net_worth_minor + from_s.total_net_worth_minor \
         FROM portfolio_daily_snapshots from_s \
         WHERE into_s.owner_id = $1 \
           AND from_s.owner_id = $2 \
           AND into_s.snapshot_date = from_s.snapshot_date \
           AND into_s.currency = from_s.currency",
    ))
    .bind(into.as_uuid())
    .bind(from.as_uuid())
    .execute(&mut *conn)
    .await?;
    if snapshots_merged.rows_affected() > 0 {
        sqlx::query(AssertSqlSafe(
            "DELETE FROM portfolio_daily_snapshots from_s \
             USING portfolio_daily_snapshots into_s \
             WHERE from_s.owner_id = $1 \
               AND into_s.owner_id = $2 \
               AND from_s.snapshot_date = into_s.snapshot_date \
               AND from_s.currency = into_s.currency",
        ))
        .bind(from.as_uuid())
        .bind(into.as_uuid())
        .execute(&mut *conn)
        .await?;
        out.snapshots = snapshots_merged.rows_affected() as i64;
    }

    // 5. One recording per day: `into`'s wins.
    sqlx::query(AssertSqlSafe(
        "DELETE FROM briefing_audio b          WHERE b.owner_id = $2            AND EXISTS (SELECT 1 FROM briefing_audio o                        WHERE o.owner_id = $1 AND o.local_date = b.local_date)",
    ))
    .bind(into.as_uuid())
    .bind(from.as_uuid())
    .execute(&mut *conn)
    .await?;

    Ok(out)
}

/// Moves every row of `table` from one owner to another, skipping rows that
/// would collide with one the destination already has.
///
/// Attempts bulk update within a savepoint first; on unique constraint violation,
/// rolls back and updates rows individually within per-row savepoints.
pub async fn move_rows(
    conn: &mut sqlx::PgConnection,
    table: &str,
    column: &str,
    from: &AccountId,
    into: &AccountId,
) -> Result<i64, sqlx::Error> {
    assert_allowed_table(table);
    assert_allowed_column(column);

    sqlx::query(AssertSqlSafe("SAVEPOINT merge_move"))
        .execute(&mut *conn)
        .await?;

    let bulk_sql = format!("UPDATE {table} SET {column} = $1 WHERE {column} = $2");
    let bulk = sqlx::query(AssertSqlSafe(bulk_sql.as_str()))
        .bind(into.as_uuid())
        .bind(from.as_uuid())
        .execute(&mut *conn)
        .await;

    match bulk {
        Ok(res) => {
            sqlx::query(AssertSqlSafe("RELEASE SAVEPOINT merge_move"))
                .execute(&mut *conn)
                .await?;
            Ok(res.rows_affected() as i64)
        }
        Err(err) if is_unique_violation(&err) => {
            sqlx::query(AssertSqlSafe("ROLLBACK TO SAVEPOINT merge_move"))
                .execute(&mut *conn)
                .await?;
            sqlx::query(AssertSqlSafe("RELEASE SAVEPOINT merge_move"))
                .execute(&mut *conn)
                .await?;

            let key = row_key(table);
            let select_keys_sql =
                format!("SELECT CAST({key} AS TEXT) FROM {table} WHERE {column} = $1");
            let keys: Vec<String> = sqlx::query_scalar(AssertSqlSafe(select_keys_sql.as_str()))
                .bind(from.as_uuid())
                .fetch_all(&mut *conn)
                .await?;

            let update_row_sql = format!(
                "UPDATE {table} SET {column} = $1 \
                 WHERE {column} = $2 AND CAST({key} AS TEXT) = $3"
            );

            let mut moved = 0i64;
            for value in keys {
                sqlx::query(AssertSqlSafe("SAVEPOINT merge_row"))
                    .execute(&mut *conn)
                    .await?;

                let one = sqlx::query(AssertSqlSafe(update_row_sql.as_str()))
                    .bind(into.as_uuid())
                    .bind(from.as_uuid())
                    .bind(&value)
                    .execute(&mut *conn)
                    .await;

                match one {
                    Ok(n) => {
                        sqlx::query(AssertSqlSafe("RELEASE SAVEPOINT merge_row"))
                            .execute(&mut *conn)
                            .await?;
                        moved += n.rows_affected() as i64;
                    }
                    Err(e) if is_unique_violation(&e) => {
                        sqlx::query(AssertSqlSafe("ROLLBACK TO SAVEPOINT merge_row"))
                            .execute(&mut *conn)
                            .await?;
                        sqlx::query(AssertSqlSafe("RELEASE SAVEPOINT merge_row"))
                            .execute(&mut *conn)
                            .await?;
                    }
                    Err(e) => return Err(e),
                }
            }
            Ok(moved)
        }
        Err(err) => Err(err),
    }
}

/// Deletes leftover rows of `table` owned by `from` (e.g. collided rows that could not move).
pub async fn delete_leftovers(
    conn: &mut sqlx::PgConnection,
    table: &str,
    column: &str,
    from: &AccountId,
) -> Result<i64, sqlx::Error> {
    assert_allowed_table(table);
    assert_allowed_column(column);

    let count_sql = format!("SELECT COUNT(*) FROM {table} WHERE {column} = $1");
    let leftover: i64 = sqlx::query_scalar(AssertSqlSafe(count_sql.as_str()))
        .bind(from.as_uuid())
        .fetch_one(&mut *conn)
        .await
        .unwrap_or(0);

    if leftover > 0 {
        let delete_sql = format!("DELETE FROM {table} WHERE {column} = $1");
        sqlx::query(AssertSqlSafe(delete_sql.as_str()))
            .bind(from.as_uuid())
            .execute(&mut *conn)
            .await?;
    }

    Ok(leftover)
}

/// Discards credential or ceremony rows associated with `from`.
pub async fn discard_account_table(
    conn: &mut sqlx::PgConnection,
    table: &str,
    from: &AccountId,
) -> Result<(), sqlx::Error> {
    assert_allowed_table(table);

    if table == "sessions" {
        sqlx::query(AssertSqlSafe(
            "UPDATE sessions SET revoked_at = CURRENT_TIMESTAMP WHERE account_id = $1",
        ))
        .bind(from.as_uuid())
        .execute(conn)
        .await?;
    } else {
        let delete_sql = format!("DELETE FROM {table} WHERE account_id = $1");
        sqlx::query(AssertSqlSafe(delete_sql.as_str()))
            .bind(from.as_uuid())
            .execute(conn)
            .await?;
    }

    Ok(())
}

/// Deletes the source account row from `accounts`.
pub async fn delete_account(
    conn: &mut sqlx::PgConnection,
    from: &AccountId,
) -> Result<(), sqlx::Error> {
    sqlx::query(AssertSqlSafe("DELETE FROM accounts WHERE id = $1"))
        .bind(from.as_uuid())
        .execute(conn)
        .await?;
    Ok(())
}

/// Counts the number of rows matching `column = $1` in `table`.
pub async fn count_rows(
    executor: impl sqlx::PgExecutor<'_>,
    table: &str,
    column: &str,
    owner: &AccountId,
) -> Result<i64, sqlx::Error> {
    assert_allowed_table(table);
    assert_allowed_column(column);

    let count_sql = format!("SELECT COUNT(*) FROM {table} WHERE {column} = $1");
    let count: i64 = sqlx::query_scalar(AssertSqlSafe(count_sql.as_str()))
        .bind(owner.as_uuid())
        .fetch_one(executor)
        .await
        .unwrap_or(0);

    Ok(count)
}

/// Counts what a merge from `from` would move across all known owned tables.
pub async fn preview(pool: &sqlx::PgPool, from: &AccountId) -> Result<MergePreview, sqlx::Error> {
    let mut preview = MergePreview::default();

    for &table in OWNED_TABLES {
        let count = count_rows(pool, table, "owner_id", from).await?;
        preview.set(table, count);
    }

    for &table in OWNER_ID_MERGED_SEPARATELY {
        let count = count_rows(pool, table, "owner_id", from).await?;
        preview.set(table, count);
    }

    Ok(preview)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;
    use std::collections::BTreeSet;

    #[test]
    #[should_panic(expected = "not in the merge allow-list")]
    fn test_unallowed_table_panics() {
        assert_allowed_table("malicious_table; DROP TABLE accounts;--");
    }

    #[test]
    #[should_panic(expected = "not an allowed merge column")]
    fn test_unallowed_column_panics() {
        assert_allowed_column("password_hash");
    }

    #[tokio::test]
    async fn test_every_merged_table_has_the_row_key_this_names() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;

        for &table in OWNED_TABLES.iter().chain(ACCOUNT_ID_FOLLOWS.iter()) {
            let key = row_key(table);
            let probe = format!("SELECT {key} FROM {table} LIMIT 0");
            sqlx::query(AssertSqlSafe(probe.as_str()))
                .execute(&pool)
                .await
                .unwrap_or_else(|err| {
                    panic!("row_key({table}) = {key} is not a column on {table}: {err}")
                });
        }
    }

    #[tokio::test]
    async fn test_a_collision_does_not_block_the_rows_that_can_move() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;

        let from = AccountId::from_seed("acct_from");
        let into = AccountId::from_seed("acct_into");

        for acc in [&from, &into] {
            sqlx::query!(
                "INSERT INTO accounts (id, display_name, status) VALUES ($1, 'test', 'active') ON CONFLICT DO NOTHING",
                acc.as_uuid()
            )
            .execute(&pool)
            .await
            .unwrap();
        }

        for (owner, name) in [(&from, "food"), (&from, "travel"), (&into, "food")] {
            sqlx::query!(
                "INSERT INTO finance_categories (owner_id, name) VALUES ($1, $2)",
                owner.as_uuid(),
                name
            )
            .execute(&pool)
            .await
            .unwrap();
        }

        let mut tx = pool.begin().await.unwrap();
        let moved = move_rows(&mut tx, "finance_categories", "owner_id", &from, &into)
            .await
            .expect("a collision must not fail the move");
        tx.commit().await.unwrap();

        assert_eq!(moved, 1, "only `travel` can move");

        let names: Vec<String> = sqlx::query_scalar!(
            "SELECT name FROM finance_categories WHERE owner_id = $1 ORDER BY name",
            into.as_uuid()
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(names, vec!["food".to_string(), "travel".to_string()]);

        let left: Vec<String> = sqlx::query_scalar!(
            "SELECT name FROM finance_categories WHERE owner_id = $1",
            from.as_uuid()
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(left, vec!["food".to_string()]);
    }

    #[tokio::test]
    async fn merge_remaps_budget_category_and_keeps_into_on_collision() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;

        let from = AccountId::generate();
        let into = AccountId::generate();
        crate::db::pool::seed_account(&pool, &from).await;
        crate::db::pool::seed_account(&pool, &into).await;

        let mut cat = std::collections::HashMap::new();
        for (owner, name) in [(&from, "food"), (&from, "travel"), (&into, "food")] {
            let id = sqlx::query_scalar!(
                "INSERT INTO finance_categories (owner_id, name) VALUES ($1, $2) RETURNING id",
                owner.as_uuid(),
                name
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            cat.insert((owner.as_uuid(), name), id);
        }
        let budgets = [
            (&from, Some(cat[&(from.as_uuid(), "food")]), 111_i64),
            (&from, Some(cat[&(from.as_uuid(), "travel")]), 222),
            (&from, None, 333),
            (&into, Some(cat[&(into.as_uuid(), "food")]), 1_000),
            (&into, None, 3_000),
        ];
        for (owner, category_id, limit) in budgets {
            sqlx::query!(
                "INSERT INTO finance_budgets (owner_id, category_id, currency, \
                 currency_exponent, limit_minor) VALUES ($1, $2, 'EUR', 2, $3)",
                owner.as_uuid(),
                category_id,
                limit
            )
            .execute(&pool)
            .await
            .unwrap();
        }

        let mut tx = pool.begin().await.unwrap();
        reconcile_colliding_owned_rows(&mut tx, &from, &into)
            .await
            .unwrap();
        move_rows(&mut tx, "finance_categories", "owner_id", &from, &into)
            .await
            .unwrap();
        move_rows(&mut tx, "finance_budgets", "owner_id", &from, &into)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let rows = sqlx::query!(
            r#"SELECT c.name AS "name?", b.limit_minor FROM finance_budgets b
               LEFT JOIN finance_categories c ON c.id = b.category_id
               WHERE b.owner_id = $1 ORDER BY c.name NULLS LAST"#,
            into.as_uuid()
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let got: Vec<(Option<String>, i64)> =
            rows.into_iter().map(|r| (r.name, r.limit_minor)).collect();
        assert_eq!(
            got,
            vec![
                (Some("food".to_string()), 1_000),
                (Some("travel".to_string()), 222),
                (None, 3_000),
            ]
        );
    }

    /// `finance_categories` is unique per exact name, so `from` can hold "Food" and
    /// "food" with a budget each; both remap to `into`'s one "food" category.
    #[tokio::test]
    async fn merge_keeps_one_budget_when_case_variant_categories_collapse() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;

        let from = AccountId::generate();
        let into = AccountId::generate();
        crate::db::pool::seed_account(&pool, &from).await;
        crate::db::pool::seed_account(&pool, &into).await;

        let mut cat = std::collections::HashMap::new();
        for (owner, name) in [(&from, "Food"), (&from, "food"), (&into, "food")] {
            let id = sqlx::query_scalar!(
                "INSERT INTO finance_categories (owner_id, name) VALUES ($1, $2) RETURNING id",
                owner.as_uuid(),
                name
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            cat.insert((owner.as_uuid(), name), id);
        }
        // `into` has the category but no budget on it, so nothing is deleted as an
        // `into` collision and both of `from`'s budgets reach the remap.
        for (name, limit) in [("Food", 111_i64), ("food", 222)] {
            sqlx::query!(
                "INSERT INTO finance_budgets (owner_id, category_id, currency, \
                 currency_exponent, limit_minor) VALUES ($1, $2, 'EUR', 2, $3)",
                from.as_uuid(),
                Some(cat[&(from.as_uuid(), name)]),
                limit
            )
            .execute(&pool)
            .await
            .unwrap();
        }

        let mut tx = pool.begin().await.unwrap();
        reconcile_colliding_owned_rows(&mut tx, &from, &into)
            .await
            .unwrap();
        move_rows(&mut tx, "finance_categories", "owner_id", &from, &into)
            .await
            .unwrap();
        move_rows(&mut tx, "finance_budgets", "owner_id", &from, &into)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let rows = sqlx::query!(
            r#"SELECT c.name AS "name?", b.limit_minor FROM finance_budgets b
               LEFT JOIN finance_categories c ON c.id = b.category_id
               WHERE b.owner_id = $1 ORDER BY c.name NULLS LAST"#,
            into.as_uuid()
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let got: Vec<(Option<String>, i64)> =
            rows.into_iter().map(|r| (r.name, r.limit_minor)).collect();
        assert_eq!(
            got.len(),
            1,
            "one budget per category and currency: {got:?}"
        );
        assert_eq!(got[0].0.as_deref(), Some("food"));
    }

    /// Every table in the schema that has `column`.
    async fn tables_with_column(pool: &PgPool, column: &str) -> BTreeSet<String> {
        sqlx::query_scalar!(
            "SELECT table_name FROM information_schema.columns \
             WHERE table_schema = current_schema() AND column_name = $1",
            column
        )
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .flatten()
        .collect()
    }

    fn declared(lists: &[&[&str]]) -> BTreeSet<String> {
        lists
            .iter()
            .flat_map(|list| list.iter())
            .map(|name| name.to_string())
            .collect()
    }

    async fn migrated_pool() -> PgPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    /// The list a merge walks must be derived from the schema, not remembered.
    ///
    /// It fell three tables behind before this existed, so a merge silently
    /// stranded a user's finance share, their audit ledger and their reaction
    /// targets on an account it then deleted. A new `owner_id` table must now be
    /// classified as following a merge or explicitly not, or this fails.
    #[tokio::test]
    async fn test_owned_tables_and_exceptions_match_the_schema() {
        let pool = migrated_pool().await;
        let in_schema = tables_with_column(&pool, "owner_id").await;
        let classified = declared(&[OWNED_TABLES, OWNER_ID_MERGED_SEPARATELY]);

        let unclassified: Vec<_> = in_schema.difference(&classified).collect();
        assert!(
            unclassified.is_empty(),
            "these tables carry owner_id but no list classifies them: {unclassified:?}"
        );

        let phantom: Vec<_> = classified.difference(&in_schema).collect();
        assert!(
            phantom.is_empty(),
            "these tables are classified but are not in the schema: {phantom:?}"
        );

        for table in OWNER_ID_MERGED_SEPARATELY {
            assert!(
                !OWNED_TABLES.contains(table),
                "{table} cannot both follow a merge and be excluded from one"
            );
        }
    }

    /// Same rule for the other ownership column. An unclassified table here leaves
    /// a row pointing at an account id that the merge is about to delete.
    #[tokio::test]
    async fn test_account_id_tables_are_all_classified() {
        let pool = migrated_pool().await;
        let in_schema = tables_with_column(&pool, "account_id").await;
        let classified = declared(&[
            ACCOUNT_ID_FOLLOWS,
            ACCOUNT_ID_DISCARDED,
            ACCOUNT_ID_UNTOUCHED,
        ]);

        let unclassified: Vec<_> = in_schema.difference(&classified).collect();
        assert!(
            unclassified.is_empty(),
            "these tables carry account_id but no list classifies them: {unclassified:?}"
        );

        let phantom: Vec<_> = classified.difference(&in_schema).collect();
        assert!(
            phantom.is_empty(),
            "these tables are classified but are not in the schema: {phantom:?}"
        );
    }
}
