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
        "matrix_links" => "mxid",
        "account_rooms" => "room_id",
        _ => "id",
    }
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(e) if e.is_unique_violation())
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

        let from = AccountId::new("acct_from");
        let into = AccountId::new("acct_into");

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
}
