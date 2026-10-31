//! Calendar feed service: subscription-token lifecycle and the iCalendar
//! document a calendar client polls.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::accounts::{hash_token, AccountId};
use crate::domain::calendar::{build_feed, CalendarInput, MAX_FEED_ITEMS};
use crate::infrastructure::postgres::repositories::{
    calendar as repo, recurring_bills, reminders, todos,
};

const TOKEN_PREFIX: &str = "mb_cal_";

#[derive(Clone)]
pub struct CalendarService {
    pool: PgPool,
}

impl CalendarService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Mints the account's feed token, replacing (and so revoking) any
    /// earlier one. The plaintext is returned here and never stored.
    pub async fn mint_token(&self, account_id: &AccountId) -> Result<String, AppError> {
        let token = format!("{TOKEN_PREFIX}{}", crate::utils::random::random_hex(32));
        repo::replace_token(&self.pool, account_id, &hash_token(&token)).await?;
        Ok(token)
    }

    /// Revokes the feed token; false when the account had none.
    pub async fn revoke_token(&self, account_id: &AccountId) -> Result<bool, AppError> {
        Ok(repo::delete_token(&self.pool, account_id).await?)
    }

    /// When the account's current token was minted, if it has one.
    pub async fn token_status(
        &self,
        account_id: &AccountId,
    ) -> Result<Option<DateTime<Utc>>, AppError> {
        Ok(repo::token_created_at(&self.pool, account_id).await?)
    }

    /// The iCalendar text for the token's owner; `None` for an unknown or
    /// revoked token.
    pub async fn ics_feed(&self, raw_token: &str) -> Result<Option<String>, AppError> {
        let Some(owner) = repo::resolve_and_touch(&self.pool, &hash_token(raw_token)).await? else {
            return Ok(None);
        };
        let limit = MAX_FEED_ITEMS as i64;
        let input = CalendarInput {
            todos: todos::list_open_dated(&self.pool, &owner, limit).await?,
            bills: recurring_bills::list_by_owner(&self.pool, &owner).await?,
            reminders: reminders::list_pending_standalone(&self.pool, &owner, limit).await?,
        };
        Ok(Some(build_feed(&input, Utc::now())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::reminders::{NewReminder, RecurrencePattern, ReminderSource};
    use crate::domain::todos::{NewTodo, TodoPriority};
    use chrono::Duration;

    async fn service_with(names: &[&str]) -> (CalendarService, PgPool) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, names).await;
        (CalendarService::new(pool.clone()), pool)
    }

    async fn add_todo(pool: &PgPool, owner: &AccountId, text: &str) -> i64 {
        let input = NewTodo {
            room_id: "web".into(),
            sender_name: "t".into(),
            text: text.into(),
            priority: TodoPriority::Medium,
            due_at: Some(Utc::now() + Duration::days(1)),
            remind_before_minutes: None,
            remind_room_id: None,
            tags: vec![],
            recurrence: RecurrencePattern::None,
            recurrence_anchor: None,
        };
        todos::create(pool, owner, &input).await.unwrap().id
    }

    #[tokio::test]
    async fn mint_then_feed_contains_owner_items_only() {
        let (svc, pool) = service_with(&["svc_cal_a", "svc_cal_b"]).await;
        let (a, b) = (
            AccountId::from_seed("svc_cal_a"),
            AccountId::from_seed("svc_cal_b"),
        );
        add_todo(&pool, &a, "alice task").await;
        add_todo(&pool, &b, "bob task").await;

        let token = svc.mint_token(&a).await.unwrap();
        assert!(token.starts_with("mb_cal_"));
        let feed = svc.ics_feed(&token).await.unwrap().expect("feed");
        assert!(feed.contains("SUMMARY:alice task"));
        assert!(!feed.contains("bob task"));
    }

    #[tokio::test]
    async fn second_mint_kills_first_token_and_revoke_ends_the_feed() {
        let (svc, _pool) = service_with(&["svc_cal_c"]).await;
        let c = AccountId::from_seed("svc_cal_c");
        let first = svc.mint_token(&c).await.unwrap();
        let second = svc.mint_token(&c).await.unwrap();
        assert!(svc.ics_feed(&first).await.unwrap().is_none());
        assert!(svc.ics_feed(&second).await.unwrap().is_some());

        assert!(svc.revoke_token(&c).await.unwrap());
        assert!(svc.ics_feed(&second).await.unwrap().is_none());
        assert!(svc.token_status(&c).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn unknown_and_empty_tokens_yield_none() {
        let (svc, _pool) = service_with(&["svc_cal_d"]).await;
        assert!(svc.ics_feed("mb_cal_nope").await.unwrap().is_none());
        assert!(svc.ics_feed("").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn sourced_reminder_is_not_in_feed_but_its_todo_is() {
        let (svc, pool) = service_with(&["svc_cal_e"]).await;
        let e = AccountId::from_seed("svc_cal_e");
        let todo_id = add_todo(&pool, &e, "has reminder").await;
        let sourced = NewReminder {
            message: "sourced ping".into(),
            trigger_at: Utc::now() + Duration::hours(2),
            room_id: "web".into(),
            owner_name: "t".into(),
            recurrence: RecurrencePattern::None,
            source: Some((ReminderSource::Todo, todo_id)),
        };
        reminders::create(&pool, &e, &sourced).await.unwrap();

        let token = svc.mint_token(&e).await.unwrap();
        let feed = svc.ics_feed(&token).await.unwrap().unwrap();
        assert!(feed.contains(&format!("UID:todo-{todo_id}@matrix-bot")));
        assert!(!feed.contains("sourced ping"));
    }

    #[tokio::test]
    async fn token_is_stored_hashed() {
        let (svc, pool) = service_with(&["svc_cal_f"]).await;
        let f = AccountId::from_seed("svc_cal_f");
        let token = svc.mint_token(&f).await.unwrap();
        let stored: String = sqlx::query_scalar!(
            "SELECT token_hash FROM account_calendar_tokens WHERE account_id = $1",
            f.as_uuid()
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_ne!(stored, token);
        assert_eq!(stored, hash_token(&token));
    }
}
