use chrono::{Months, NaiveDateTime, TimeZone, Utc};
use sqlx::SqlitePool;

use super::models::{CreateBillPayload, RecurringBill, UpdateBillPayload};
use crate::modules::finance::models::{Transaction, TransactionSource};
use crate::modules::finance::repository::FinanceRepository;

pub struct RecurringBillRepository;

impl RecurringBillRepository {
    pub async fn create(
        pool: &SqlitePool,
        room_id: &str,
        owner_id: &str,
        sender_name: &str,
        payload: CreateBillPayload,
    ) -> Result<RecurringBill, sqlx::Error> {
        let currency_exponent = payload.currency_exponent.unwrap_or(2);
        let frequency = payload.frequency.unwrap_or_else(|| "monthly".to_string());
        let auto_pay = payload.auto_pay.unwrap_or(false);

        let res = sqlx::query(
            r#"
            INSERT INTO recurring_bills (
                room_id, owner_id, sender_name, name, category,
                amount_minor, currency, currency_exponent, frequency,
                due_day, next_due_at, total_installments, auto_pay, notes,
                remind_before_minutes, remind_room_id
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?);
            "#,
        )
        .bind(room_id)
        .bind(owner_id)
        .bind(sender_name)
        .bind(&payload.name)
        .bind(&payload.category)
        .bind(payload.amount_minor)
        .bind(&payload.currency)
        .bind(currency_exponent)
        .bind(&frequency)
        .bind(payload.due_day)
        .bind(&payload.next_due_at)
        .bind(payload.total_installments)
        .bind(auto_pay)
        .bind(&payload.notes)
        .bind(payload.remind_before_minutes)
        .bind(&payload.remind_room_id)
        .execute(pool)
        .await?;

        let id = res.last_insert_rowid();

        Self::get_by_id(pool, id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn get_by_id(pool: &SqlitePool, id: i64) -> Result<Option<RecurringBill>, sqlx::Error> {
        sqlx::query_as::<_, RecurringBill>("SELECT * FROM recurring_bills WHERE id = ?;")
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn list_by_owner(
        pool: &SqlitePool,
        owner_id: &str,
    ) -> Result<Vec<RecurringBill>, sqlx::Error> {
        sqlx::query_as::<_, RecurringBill>(
            "SELECT * FROM recurring_bills WHERE owner_id = ? ORDER BY next_due_at ASC;",
        )
        .bind(owner_id)
        .fetch_all(pool)
        .await
    }

    pub async fn update(
        pool: &SqlitePool,
        id: i64,
        owner_id: &str,
        payload: UpdateBillPayload,
    ) -> Result<RecurringBill, sqlx::Error> {
        let existing = Self::get_by_id(pool, id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)?;

        if existing.owner_id != owner_id {
            return Err(sqlx::Error::RowNotFound);
        }

        let name = payload.name.unwrap_or(existing.name);
        let category = payload.category.unwrap_or(existing.category);
        let amount_minor = payload.amount_minor.unwrap_or(existing.amount_minor);
        let currency = payload.currency.unwrap_or(existing.currency);
        let frequency = payload.frequency.unwrap_or(existing.frequency);
        let due_day = payload.due_day.or(existing.due_day);
        let next_due_at = payload.next_due_at.unwrap_or(existing.next_due_at);
        let total_installments = payload.total_installments.or(existing.total_installments);
        let paid_installments = payload.paid_installments.unwrap_or(existing.paid_installments);
        let auto_pay = payload.auto_pay.unwrap_or(existing.auto_pay);
        let status = payload.status.unwrap_or(existing.status);
        let notes = payload.notes.or(existing.notes);
        // `Option<Option<i64>>`: absent keeps the current lead, `null` clears it.
        let remind_before_minutes = payload
            .remind_before_minutes
            .unwrap_or(existing.remind_before_minutes);
        let remind_room_id = payload.remind_room_id.or(existing.remind_room_id);

        sqlx::query(
            r#"
            UPDATE recurring_bills
            SET name = ?, category = ?, amount_minor = ?, currency = ?, frequency = ?,
                due_day = ?, next_due_at = ?, total_installments = ?, paid_installments = ?,
                auto_pay = ?, status = ?, notes = ?, remind_before_minutes = ?,
                remind_room_id = ?, updated_at = CURRENT_TIMESTAMP
            WHERE id = ? AND owner_id = ?;
            "#,
        )
        .bind(&name)
        .bind(&category)
        .bind(amount_minor)
        .bind(&currency)
        .bind(&frequency)
        .bind(due_day)
        .bind(&next_due_at)
        .bind(total_installments)
        .bind(paid_installments)
        .bind(auto_pay)
        .bind(&status)
        .bind(&notes)
        .bind(remind_before_minutes)
        .bind(&remind_room_id)
        .bind(id)
        .bind(owner_id)
        .execute(pool)
        .await?;

        Self::get_by_id(pool, id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn delete(pool: &SqlitePool, id: i64, owner_id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM recurring_bills WHERE id = ? AND owner_id = ?;")
            .bind(id)
            .bind(owner_id)
            .execute(pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn mark_paid(
        pool: &SqlitePool,
        id: i64,
        owner_id: &str,
    ) -> Result<(RecurringBill, Transaction), sqlx::Error> {
        let bill = Self::get_by_id(pool, id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)?;

        if bill.owner_id != owner_id {
            return Err(sqlx::Error::RowNotFound);
        }

        let new_paid_installments = bill.paid_installments + 1;
        let is_completed =
            matches!(bill.total_installments, Some(total) if new_paid_installments >= total);

        let new_status = if is_completed {
            "completed".to_string()
        } else {
            bill.status
        };

        let next_due = advance_due_date(&bill.next_due_at, &bill.frequency);

        sqlx::query(
            r#"
            UPDATE recurring_bills
            SET paid_installments = ?, next_due_at = ?, status = ?, updated_at = CURRENT_TIMESTAMP
            WHERE id = ? AND owner_id = ?;
            "#,
        )
        .bind(new_paid_installments)
        .bind(&next_due)
        .bind(&new_status)
        .bind(id)
        .bind(owner_id)
        .execute(pool)
        .await?;

        let updated = Self::get_by_id(pool, id)
            .await?
            .ok_or(sqlx::Error::RowNotFound)?;

        let category_id = FinanceRepository::find_or_create_category(pool, &bill.owner_id, &bill.category)
            .await
            .ok()
            .flatten()
            .map(|c| c.id);

        let note = format!("Recurring bill: {}", bill.name);
        let tx = FinanceRepository::create_transaction(
            pool,
            &bill.room_id,
            &bill.owner_id,
            &bill.sender_name,
            "expense",
            bill.amount_minor,
            &bill.currency,
            bill.currency_exponent as i64,
            category_id,
            Some(&note),
            Some(TransactionSource::RecurringBill.as_str()),
            Some(bill.id),
            Utc::now(),
        )
        .await
        .map_err(|e| sqlx::Error::Protocol(format!("Failed to record bill transaction: {}", e)))?;

        Ok((updated, tx))
    }
}

pub fn advance_due_date(current_due_at: &str, frequency: &str) -> String {
    let parsed = NaiveDateTime::parse_from_str(current_due_at, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(current_due_at, "%Y-%m-%dT%H:%M:%S"))
        .or_else(|_| NaiveDateTime::parse_from_str(&format!("{} 00:00:00", current_due_at), "%Y-%m-%d %H:%M:%S"));

    let dt = match parsed {
        Ok(ndt) => Utc.from_utc_datetime(&ndt),
        Err(_) => Utc::now(),
    };

    let next_dt = match frequency.to_lowercase().as_str() {
        "daily" => dt + chrono::Duration::days(1),
        "weekly" => dt + chrono::Duration::weeks(1),
        "quarterly" => dt.checked_add_months(Months::new(3)).unwrap_or(dt),
        "yearly" => dt.checked_add_months(Months::new(12)).unwrap_or(dt),
        _ => dt.checked_add_months(Months::new(1)).unwrap_or(dt), // Default monthly
    };

    next_dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_recurring_bill_crud() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::schema::run_migrations(&pool).await.unwrap();

        let payload = CreateBillPayload {
            name: "Netflix Subscription".to_string(),
            category: "subscription".to_string(),
            amount_minor: 1599,
            currency: "USD".to_string(),
            currency_exponent: Some(2),
            frequency: Some("monthly".to_string()),
            due_day: Some(15),
            next_due_at: "2026-09-15 00:00:00".to_string(),
            total_installments: None,
            auto_pay: Some(true),
            notes: Some("Standard plan".to_string()),
            remind_before_minutes: None,
            remind_room_id: None,
        };

        let created = RecurringBillRepository::create(&pool, "web", "@alice:a", "Alice", payload)
            .await
            .unwrap();

        assert_eq!(created.name, "Netflix Subscription");
        assert_eq!(created.amount_minor, 1599);
        assert_eq!(created.category, "subscription");

        let list = RecurringBillRepository::list_by_owner(&pool, "@alice:a")
            .await
            .unwrap();
        assert_eq!(list.len(), 1);

        let (paid, tx) = RecurringBillRepository::mark_paid(&pool, created.id, "@alice:a")
            .await
            .unwrap();
        assert_eq!(paid.paid_installments, 1);
        assert_eq!(paid.next_due_at, "2026-10-15 00:00:00");
        assert_eq!(tx.amount_minor, 1599);
        assert_eq!(tx.direction, "expense");
        assert_eq!(tx.source_kind.as_deref(), Some("recurring_bill"));
        assert_eq!(tx.source_id, Some(created.id));

        let deleted = RecurringBillRepository::delete(&pool, created.id, "@alice:a")
            .await
            .unwrap();
        assert!(deleted);
    }
}
