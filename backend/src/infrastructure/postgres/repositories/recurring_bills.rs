//! PostgreSQL repository for recurring bills.

use sqlx::PgExecutor;

use crate::domain::accounts::AccountId;
use crate::domain::recurring_bills::{NewRecurringBill, RecurringBill, RecurringBillPatch};

#[derive(Debug, sqlx::FromRow)]
struct RecurringBillRow {
    id: i64,
    room_id: String,
    owner_id: String,
    sender_name: String,
    name: String,
    category: String,
    amount_minor: i64,
    currency: String,
    currency_exponent: i64,
    frequency: String,
    due_day: Option<i32>,
    next_due_at: String,
    total_installments: Option<i32>,
    paid_installments: i32,
    auto_pay: bool,
    status: String,
    notes: Option<String>,
    remind_before_minutes: Option<i64>,
    remind_room_id: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

impl From<RecurringBillRow> for RecurringBill {
    fn from(r: RecurringBillRow) -> Self {
        Self {
            id: r.id,
            room_id: r.room_id,
            owner_id: AccountId::new(r.owner_id),
            sender_name: r.sender_name,
            name: r.name,
            category: r.category,
            amount_minor: r.amount_minor,
            currency: r.currency,
            currency_exponent: r.currency_exponent,
            frequency: r.frequency,
            due_day: r.due_day,
            next_due_at: r.next_due_at,
            total_installments: r.total_installments,
            paid_installments: r.paid_installments,
            auto_pay: r.auto_pay,
            status: r.status,
            notes: r.notes,
            remind_before_minutes: r.remind_before_minutes,
            remind_room_id: r.remind_room_id,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

pub async fn create<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    input: &NewRecurringBill,
) -> Result<RecurringBill, sqlx::Error> {
    let row = sqlx::query_as!(
        RecurringBillRow,
        r#"
        INSERT INTO recurring_bills (
            room_id, owner_id, sender_name, name, category,
            amount_minor, currency, currency_exponent, frequency,
            due_day, next_due_at, total_installments, paid_installments,
            auto_pay, status, notes, remind_before_minutes, remind_room_id
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 0, $13, 'active', $14, $15, $16)
        RETURNING
            id, room_id, owner_id, sender_name, name, category,
            amount_minor, currency, currency_exponent, frequency,
            due_day, next_due_at, total_installments, paid_installments,
            auto_pay, status, notes, remind_before_minutes, remind_room_id,
            created_at, updated_at
        "#,
        input.room_id,
        owner_id.as_str(),
        input.sender_name,
        input.name,
        input.category,
        input.amount_minor,
        input.currency,
        input.currency_exponent,
        input.frequency,
        input.due_day,
        input.next_due_at,
        input.total_installments,
        input.auto_pay,
        input.notes,
        input.remind_before_minutes,
        input.remind_room_id
    )
    .fetch_one(conn)
    .await?;

    Ok(row.into())
}

pub async fn find_by_id<'e>(
    conn: impl PgExecutor<'e>,
    id: i64,
) -> Result<Option<RecurringBill>, sqlx::Error> {
    let row = sqlx::query_as!(
        RecurringBillRow,
        r#"
        SELECT
            id, room_id, owner_id, sender_name, name, category,
            amount_minor, currency, currency_exponent, frequency,
            due_day, next_due_at, total_installments, paid_installments,
            auto_pay, status, notes, remind_before_minutes, remind_room_id,
            created_at, updated_at
        FROM recurring_bills
        WHERE id = $1
        "#,
        id
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn find_owned<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    id: i64,
) -> Result<Option<RecurringBill>, sqlx::Error> {
    let row = sqlx::query_as!(
        RecurringBillRow,
        r#"
        SELECT
            id, room_id, owner_id, sender_name, name, category,
            amount_minor, currency, currency_exponent, frequency,
            due_day, next_due_at, total_installments, paid_installments,
            auto_pay, status, notes, remind_before_minutes, remind_room_id,
            created_at, updated_at
        FROM recurring_bills
        WHERE id = $1 AND owner_id = $2
        "#,
        id,
        owner_id.as_str()
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn list_by_owner<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
) -> Result<Vec<RecurringBill>, sqlx::Error> {
    let rows = sqlx::query_as!(
        RecurringBillRow,
        r#"
        SELECT
            id, room_id, owner_id, sender_name, name, category,
            amount_minor, currency, currency_exponent, frequency,
            due_day, next_due_at, total_installments, paid_installments,
            auto_pay, status, notes, remind_before_minutes, remind_room_id,
            created_at, updated_at
        FROM recurring_bills
        WHERE owner_id = $1
        ORDER BY next_due_at ASC
        "#,
        owner_id.as_str()
    )
    .fetch_all(conn)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn update<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    id: i64,
    patch: &RecurringBillPatch,
) -> Result<Option<RecurringBill>, sqlx::Error> {
    let has_due_day = patch.due_day.is_some();
    let due_day_val = patch.due_day;
    let has_installments = patch.total_installments.is_some();
    let installments_val = patch.total_installments;
    let has_notes = patch.notes.is_some();
    let notes_val = patch.notes.as_deref();
    let has_remind = patch.remind_before_minutes.is_some();
    let remind_val = patch.remind_before_minutes.flatten();
    let has_remind_room = patch.remind_room_id.is_some();
    let remind_room_val = patch.remind_room_id.as_deref();

    let row = sqlx::query_as!(
        RecurringBillRow,
        r#"
        UPDATE recurring_bills
        SET
            name = COALESCE($1, name),
            category = COALESCE($2, category),
            amount_minor = COALESCE($3, amount_minor),
            currency = COALESCE($4, currency),
            frequency = COALESCE($5, frequency),
            due_day = CASE WHEN $6 THEN $7 ELSE due_day END,
            next_due_at = COALESCE($8, next_due_at),
            total_installments = CASE WHEN $9 THEN $10 ELSE total_installments END,
            paid_installments = COALESCE($11, paid_installments),
            auto_pay = COALESCE($12, auto_pay),
            status = COALESCE($13, status),
            notes = CASE WHEN $14 THEN $15 ELSE notes END,
            remind_before_minutes = CASE WHEN $16 THEN $17 ELSE remind_before_minutes END,
            remind_room_id = CASE WHEN $18 THEN $19 ELSE remind_room_id END,
            updated_at = (NOW()::text)
        WHERE id = $20 AND owner_id = $21
        RETURNING
            id, room_id, owner_id, sender_name, name, category,
            amount_minor, currency, currency_exponent, frequency,
            due_day, next_due_at, total_installments, paid_installments,
            auto_pay, status, notes, remind_before_minutes, remind_room_id,
            created_at, updated_at
        "#,
        patch.name.as_deref(),
        patch.category.as_deref(),
        patch.amount_minor,
        patch.currency.as_deref(),
        patch.frequency.as_deref(),
        has_due_day,
        due_day_val,
        patch.next_due_at.as_deref(),
        has_installments,
        installments_val,
        patch.paid_installments,
        patch.auto_pay,
        patch.status.as_deref(),
        has_notes,
        notes_val,
        has_remind,
        remind_val,
        has_remind_room,
        remind_room_val,
        id,
        owner_id.as_str()
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn delete<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    id: i64,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        r#"DELETE FROM recurring_bills WHERE id = $1 AND owner_id = $2"#,
        id,
        owner_id.as_str()
    )
    .execute(conn)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn record_payment<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    id: i64,
    next_due_at: &str,
    status: &str,
    new_paid_installments: i32,
) -> Result<Option<RecurringBill>, sqlx::Error> {
    let row = sqlx::query_as!(
        RecurringBillRow,
        r#"
        UPDATE recurring_bills
        SET
            paid_installments = $1,
            next_due_at = $2,
            status = $3,
            updated_at = (NOW()::text)
        WHERE id = $4 AND owner_id = $5
        RETURNING
            id, room_id, owner_id, sender_name, name, category,
            amount_minor, currency, currency_exponent, frequency,
            due_day, next_due_at, total_installments, paid_installments,
            auto_pay, status, notes, remind_before_minutes, remind_room_id,
            created_at, updated_at
        "#,
        new_paid_installments,
        next_due_at,
        status,
        id,
        owner_id.as_str()
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_test_db() -> crate::db::DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    #[tokio::test]
    async fn test_recurring_bill_repository_crud() {
        let pool = setup_test_db().await;
        let owner = AccountId::new("acct_alice");

        let input = NewRecurringBill {
            room_id: "!room:bills".to_string(),
            sender_name: "alice".to_string(),
            name: "Spotify".to_string(),
            category: "Entertainment".to_string(),
            amount_minor: 999,
            currency: "USD".to_string(),
            currency_exponent: 2,
            frequency: "monthly".to_string(),
            due_day: Some(1),
            next_due_at: "2026-06-01 00:00:00".to_string(),
            total_installments: None,
            auto_pay: true,
            notes: Some("Family plan".to_string()),
            remind_before_minutes: Some(1440),
            remind_room_id: None,
        };

        let created = create(&pool, &owner, &input).await.unwrap();
        assert_eq!(created.name, "Spotify");
        assert_eq!(created.amount_minor, 999);
        assert_eq!(created.paid_installments, 0);

        let found = find_owned(&pool, &owner, created.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, created.id);

        let patch = RecurringBillPatch {
            amount_minor: Some(1199),
            ..Default::default()
        };
        let updated = update(&pool, &owner, created.id, &patch)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.amount_minor, 1199);

        let list = list_by_owner(&pool, &owner).await.unwrap();
        assert_eq!(list.len(), 1);

        let paid = record_payment(
            &pool,
            &owner,
            created.id,
            "2026-07-01 00:00:00",
            "active",
            1,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(paid.paid_installments, 1);
        assert_eq!(paid.next_due_at, "2026-07-01 00:00:00");

        let deleted = delete(&pool, &owner, created.id).await.unwrap();
        assert!(deleted);
    }
}
