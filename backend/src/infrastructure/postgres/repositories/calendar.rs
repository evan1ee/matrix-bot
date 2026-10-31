//! PostgreSQL repository for calendar feed subscription tokens.
//!
//! The agenda rows themselves come from the todo, reminder and recurring-bill
//! repositories; this file only owns who may read the feed.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;

use crate::domain::accounts::AccountId;

/// Stores `token_hash` as the account's only feed token, replacing any
/// previous one (so the old subscription URL stops working at once).
pub async fn replace_token<'e>(
    conn: impl PgExecutor<'e>,
    account_id: &AccountId,
    token_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        INSERT INTO account_calendar_tokens (account_id, token_hash, created_at, last_accessed_at)
        VALUES ($1, $2, now(), NULL)
        ON CONFLICT (account_id) DO UPDATE
            SET token_hash = EXCLUDED.token_hash,
                created_at = now(),
                last_accessed_at = NULL
        "#,
        account_id.as_uuid(),
        token_hash
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Deletes the account's token; false when it had none.
pub async fn delete_token<'e>(
    conn: impl PgExecutor<'e>,
    account_id: &AccountId,
) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!(
        "DELETE FROM account_calendar_tokens WHERE account_id = $1",
        account_id.as_uuid()
    )
    .execute(conn)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// When the account's current token was minted, if it has one.
pub async fn token_created_at<'e>(
    conn: impl PgExecutor<'e>,
    account_id: &AccountId,
) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT created_at FROM account_calendar_tokens WHERE account_id = $1",
        account_id.as_uuid()
    )
    .fetch_optional(conn)
    .await
}

/// Resolves a token hash to its account and records the access, touching
/// `last_accessed_at` at most once an hour so a polling client does not write
/// on every request.
pub async fn resolve_and_touch<'e>(
    conn: impl PgExecutor<'e>,
    token_hash: &str,
) -> Result<Option<AccountId>, sqlx::Error> {
    let row = sqlx::query_scalar!(
        r#"
        WITH hit AS (
            SELECT account_id FROM account_calendar_tokens WHERE token_hash = $1
        ), touched AS (
            UPDATE account_calendar_tokens t
               SET last_accessed_at = now()
              FROM hit
             WHERE t.account_id = hit.account_id
               AND (t.last_accessed_at IS NULL OR t.last_accessed_at < now() - interval '1 hour')
        )
        SELECT account_id AS "account_id!" FROM hit
        "#,
        token_hash
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(AccountId::from_uuid))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::reminders::{NewReminder, RecurrencePattern, ReminderSource};
    use crate::domain::todos::{NewTodo, TodoPriority};
    use crate::infrastructure::postgres::repositories::{reminders, todos};
    use chrono::Duration;

    async fn pool_with(names: &[&str]) -> sqlx::PgPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, names).await;
        pool
    }

    fn todo(text: &str, due: Option<DateTime<Utc>>) -> NewTodo {
        NewTodo {
            room_id: "web".into(),
            sender_name: "t".into(),
            text: text.into(),
            priority: TodoPriority::Medium,
            due_at: due,
            remind_before_minutes: None,
            remind_room_id: None,
            tags: vec![],
            recurrence: RecurrencePattern::None,
            recurrence_anchor: None,
        }
    }

    fn reminder(source: Option<(ReminderSource, i64)>) -> NewReminder {
        NewReminder {
            message: "ping".into(),
            trigger_at: Utc::now() + Duration::days(1),
            room_id: "web".into(),
            owner_name: "t".into(),
            recurrence: RecurrencePattern::None,
            source,
        }
    }

    #[tokio::test]
    async fn replace_token_invalidates_previous_hash() {
        let pool = pool_with(&["cal_a"]).await;
        let a = AccountId::from_seed("cal_a");
        replace_token(&pool, &a, "h1").await.unwrap();
        replace_token(&pool, &a, "h2").await.unwrap();
        assert_eq!(resolve_and_touch(&pool, "h1").await.unwrap(), None);
        assert_eq!(resolve_and_touch(&pool, "h2").await.unwrap(), Some(a));
    }

    #[tokio::test]
    async fn resolve_unknown_hash_is_none_and_delete_reports_absence() {
        let pool = pool_with(&["cal_b"]).await;
        let b = AccountId::from_seed("cal_b");
        assert_eq!(resolve_and_touch(&pool, "nope").await.unwrap(), None);
        assert!(!delete_token(&pool, &b).await.unwrap());
        replace_token(&pool, &b, "h").await.unwrap();
        assert!(token_created_at(&pool, &b).await.unwrap().is_some());
        assert!(delete_token(&pool, &b).await.unwrap());
        assert_eq!(resolve_and_touch(&pool, "h").await.unwrap(), None);
    }

    #[tokio::test]
    async fn touch_is_throttled_to_once_an_hour() {
        let pool = pool_with(&["cal_c"]).await;
        let c = AccountId::from_seed("cal_c");
        replace_token(&pool, &c, "h").await.unwrap();
        let touched = |pool: sqlx::PgPool| async move {
            sqlx::query_scalar!(
                "SELECT last_accessed_at FROM account_calendar_tokens WHERE token_hash = 'h'"
            )
            .fetch_one(&pool)
            .await
            .unwrap()
        };
        assert!(touched(pool.clone()).await.is_none());
        resolve_and_touch(&pool, "h").await.unwrap();
        let first = touched(pool.clone()).await.expect("first access recorded");
        resolve_and_touch(&pool, "h").await.unwrap();
        assert_eq!(touched(pool.clone()).await, Some(first));
    }

    #[tokio::test]
    async fn cascade_delete_removes_token_with_account() {
        let pool = pool_with(&["cal_d"]).await;
        let d = AccountId::from_seed("cal_d");
        replace_token(&pool, &d, "h").await.unwrap();
        sqlx::query!("DELETE FROM accounts WHERE id = $1", d.as_uuid())
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(resolve_and_touch(&pool, "h").await.unwrap(), None);
    }

    #[tokio::test]
    async fn todo_query_is_owner_scoped_and_skips_completed_and_undated() {
        let pool = pool_with(&["cal_e", "cal_f"]).await;
        let (e, f) = (AccountId::from_seed("cal_e"), AccountId::from_seed("cal_f"));
        let soon = Some(Utc::now() + Duration::days(1));
        let kept = todos::create(&pool, &e, &todo("kept", soon)).await.unwrap();
        let done = todos::create(&pool, &e, &todo("done", soon)).await.unwrap();
        todos::set_completed(&pool, done.id, true).await.unwrap();
        todos::create(&pool, &e, &todo("undated", None))
            .await
            .unwrap();
        todos::create(&pool, &f, &todo("theirs", soon))
            .await
            .unwrap();

        let got = todos::list_open_dated(&pool, &e, 10).await.unwrap();
        assert_eq!(got.iter().map(|t| t.id).collect::<Vec<_>>(), vec![kept.id]);
    }

    #[tokio::test]
    async fn reminder_query_excludes_sourced_and_non_pending() {
        let pool = pool_with(&["cal_g", "cal_h"]).await;
        let (g, h) = (AccountId::from_seed("cal_g"), AccountId::from_seed("cal_h"));
        let plain = reminders::create(&pool, &g, &reminder(None)).await.unwrap();
        reminders::create(&pool, &g, &reminder(Some((ReminderSource::Todo, 1))))
            .await
            .unwrap();
        let cancelled = reminders::create(&pool, &g, &reminder(None)).await.unwrap();
        reminders::cancel(&pool, &g, cancelled.id).await.unwrap();
        reminders::create(&pool, &h, &reminder(None)).await.unwrap();

        let got = reminders::list_pending_standalone(&pool, &g, 10)
            .await
            .unwrap();
        assert_eq!(got.iter().map(|r| r.id).collect::<Vec<_>>(), vec![plain.id]);
    }
}
