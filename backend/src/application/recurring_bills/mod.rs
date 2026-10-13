//! Recurring bills application service.

use chrono::Utc;
use sqlx::PgPool;

use crate::application::finance::FinanceService;
use crate::application::reminders::ReminderService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::finance::money::{exponent_for, format_amount, rescale_minor};
use crate::domain::finance::{Direction, NewTransaction, Transaction, TransactionSource};
use crate::domain::recurring_bills::{
    advance_due_date_with_due_day, reminder_room, wanted_trigger, NewRecurringBill, RecurringBill,
    RecurringBillPatch,
};
use crate::domain::reminders::{ReminderPlan, ReminderSource, SyncOutcome};
use crate::infrastructure::postgres::repositories::{self, outbox};

#[derive(Clone)]
pub struct RecurringBillsService {
    pool: PgPool,
    reminders: ReminderService,
    finance: FinanceService,
}

impl RecurringBillsService {
    pub fn new(pool: PgPool, reminders: ReminderService, finance: FinanceService) -> Self {
        Self {
            pool,
            reminders,
            finance,
        }
    }

    pub async fn create(
        &self,
        owner_id: &AccountId,
        input: NewRecurringBill,
    ) -> Result<(RecurringBill, SyncOutcome), AppError> {
        input.validate()?;

        let mut tx = self.pool.begin().await?;
        let bill = repositories::recurring_bills::create(&mut *tx, owner_id, &input).await?;
        outbox::publish(&mut tx, &SystemEvent::RecurringBillCreated(bill.clone())).await?;
        tx.commit().await?;

        let sync_outcome = self.sync_reminder_for_bill(&bill).await?;

        Ok((bill, sync_outcome))
    }

    pub async fn find_by_id(&self, id: i64) -> Result<Option<RecurringBill>, AppError> {
        let bill = repositories::recurring_bills::find_by_id(&self.pool, id).await?;
        Ok(bill)
    }

    pub async fn find_owned(
        &self,
        owner_id: &AccountId,
        id: i64,
    ) -> Result<Option<RecurringBill>, AppError> {
        let bill = repositories::recurring_bills::find_owned(&self.pool, owner_id, id).await?;
        Ok(bill)
    }

    pub async fn list_by_owner(
        &self,
        owner_id: &AccountId,
    ) -> Result<Vec<RecurringBill>, AppError> {
        let bills = repositories::recurring_bills::list_by_owner(&self.pool, owner_id).await?;
        Ok(bills)
    }

    pub async fn update(
        &self,
        owner_id: &AccountId,
        id: i64,
        mut patch: RecurringBillPatch,
    ) -> Result<(RecurringBill, SyncOutcome), AppError> {
        patch.validate()?;

        let mut tx = self.pool.begin().await?;
        if patch.currency.is_some() && patch.amount_minor.is_none() {
            let existing =
                repositories::recurring_bills::find_owned_for_update(&mut *tx, owner_id, id)
                    .await?
                    .ok_or_else(|| {
                        AppError::NotFound("Recurring bill not found or access denied".to_string())
                    })?;
            if let Some(ref new_curr) = patch.currency {
                let new_exp = exponent_for(new_curr);
                if new_exp as i64 != existing.currency_exponent {
                    let old_exp = existing.currency_exponent.clamp(0, 8) as u32;
                    let rescaled = rescale_minor(existing.amount_minor, old_exp, new_exp)
                        .map_err(|e| {
                            let formatted = format_amount(existing.amount_minor, old_exp);
                            AppError::Validation(format!(
                                "Stored bill amount {formatted} is not valid for currency {new_curr}: {e}"
                            ))
                        })?;
                    patch.amount_minor = Some(rescaled);
                }
            }
        }
        let updated = repositories::recurring_bills::update(&mut *tx, owner_id, id, &patch)
            .await?
            .ok_or_else(|| {
                AppError::NotFound("Recurring bill not found or access denied".to_string())
            })?;
        outbox::publish(&mut tx, &SystemEvent::RecurringBillUpdated(updated.clone())).await?;
        tx.commit().await?;

        let sync_outcome = self.sync_reminder_for_bill(&updated).await?;

        Ok((updated, sync_outcome))
    }

    pub async fn delete(&self, owner_id: &AccountId, id: i64) -> Result<(), AppError> {
        let _existing = self.find_owned(owner_id, id).await?.ok_or_else(|| {
            AppError::NotFound("Recurring bill not found or access denied".to_string())
        })?;

        self.reminders
            .cancel_due_reminder(ReminderSource::RecurringBill, id)
            .await?;

        let mut tx = self.pool.begin().await?;
        repositories::recurring_bills::delete(&mut *tx, owner_id, id).await?;
        let event = SystemEvent::RecurringBillDeleted {
            id,
            owner_id: *owner_id,
        };
        outbox::publish(&mut tx, &event).await?;
        tx.commit().await?;

        Ok(())
    }

    pub async fn mark_paid(
        &self,
        owner_id: &AccountId,
        id: i64,
    ) -> Result<(RecurringBill, Transaction, SyncOutcome), AppError> {
        let mut db_tx = self.pool.begin().await?;
        let bill = repositories::recurring_bills::find_owned_for_update(&mut *db_tx, owner_id, id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound("Recurring bill not found or access denied".to_string())
            })?;

        if bill.status.eq_ignore_ascii_case("completed") {
            return Err(AppError::Validation(
                "Recurring bill is already completed".to_string(),
            ));
        }

        let new_paid_installments = bill.paid_installments + 1;
        let is_completed =
            matches!(bill.total_installments, Some(total) if new_paid_installments >= total);

        let new_status = if is_completed {
            "completed"
        } else {
            &bill.status
        };

        let next_due =
            advance_due_date_with_due_day(&bill.next_due_at, &bill.frequency, bill.due_day);

        let updated = repositories::recurring_bills::record_payment(
            &mut *db_tx,
            owner_id,
            id,
            &next_due,
            new_status,
            new_paid_installments,
        )
        .await?
        .ok_or_else(|| {
            AppError::NotFound("Recurring bill not found or access denied".to_string())
        })?;
        outbox::publish(
            &mut db_tx,
            &SystemEvent::RecurringBillUpdated(updated.clone()),
        )
        .await?;

        let note = format!("Recurring bill: {}", bill.name);
        let tx = self
            .finance
            .create_transaction_in_tx(
                &mut db_tx,
                &bill.owner_id,
                NewTransaction {
                    direction: Direction::Expense,
                    amount_minor: bill.amount_minor,
                    currency: &bill.currency,
                    currency_exponent: bill.currency_exponent,
                    category: Some(&bill.category),
                    note: Some(&note),
                    receipt_storage_key: None,
                    room_id: &bill.room_id,
                    sender_name: &bill.sender_name,
                    occurred_at: Utc::now(),
                    source: Some((TransactionSource::RecurringBill.as_str(), bill.id)),
                    create_missing_category: true,
                },
            )
            .await?;

        db_tx.commit().await?;

        let sync_outcome = self.sync_reminder_for_bill(&updated).await?;

        Ok((updated, tx, sync_outcome))
    }

    async fn sync_reminder_for_bill(&self, bill: &RecurringBill) -> Result<SyncOutcome, AppError> {
        let Some(room_id) = reminder_room(bill) else {
            return self
                .reminders
                .cancel_due_reminder(ReminderSource::RecurringBill, bill.id)
                .await;
        };

        let formatted_amount =
            format_amount(bill.amount_minor, bill.currency_exponent.clamp(0, 8) as u32);
        let plan = ReminderPlan {
            source: ReminderSource::RecurringBill,
            source_id: bill.id,
            owner_id: bill.owner_id,
            owner_name: bill.sender_name.clone(),
            room_id: room_id.to_string(),
            message: format!(
                "Bill #{} is due: {} ({} {})",
                bill.id, bill.name, bill.currency, formatted_amount
            ),
            trigger_at: wanted_trigger(bill),
        };

        self.reminders.sync_due_reminder(&plan).await
    }
}

#[cfg(test)]
mod tests {
    use crate::application::AppServices;
    use crate::domain::accounts::AccountId;
    use crate::domain::recurring_bills::NewRecurringBill;
    use crate::infrastructure::postgres::repositories;

    /// Given a bill, When it is deleted, Then RecurringBillDeleted is in the
    /// outbox.
    #[tokio::test]
    async fn delete_writes_recurring_bill_deleted_to_the_outbox() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed("bill_outbox_delete");
        crate::db::pool::seed_account(&pool, &owner).await;
        let services = AppServices::new(&pool);
        let (bill, _) = services
            .recurring_bills
            .create(
                &owner,
                NewRecurringBill {
                    room_id: "web".to_string(),
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
                    auto_pay: false,
                    notes: None,
                    remind_before_minutes: None,
                    remind_room_id: None,
                },
            )
            .await
            .unwrap();

        services
            .recurring_bills
            .delete(&owner, bill.id)
            .await
            .unwrap();

        assert_eq!(
            repositories::outbox::event_types_for(&pool, &owner)
                .await
                .unwrap(),
            vec!["RecurringBillCreated", "RecurringBillDeleted"]
        );
    }

    #[tokio::test]
    async fn mark_paid_refuses_completed_bill_and_update_syncs_exponent() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed("bill_mark_paid_m4");
        crate::db::pool::seed_account(&pool, &owner).await;
        let services = AppServices::new(&pool);
        let (bill, _) = services
            .recurring_bills
            .create(
                &owner,
                NewRecurringBill {
                    room_id: "web".to_string(),
                    sender_name: "alice".to_string(),
                    name: "Laptop installment".to_string(),
                    category: "installment".to_string(),
                    amount_minor: 1500,
                    currency: "USD".to_string(),
                    currency_exponent: 2,
                    frequency: "monthly".to_string(),
                    due_day: Some(31),
                    next_due_at: "2026-01-31 00:00:00".to_string(),
                    total_installments: Some(1),
                    auto_pay: false,
                    notes: None,
                    remind_before_minutes: None,
                    remind_room_id: None,
                },
            )
            .await
            .unwrap();

        let (paid, _, _) = services
            .recurring_bills
            .mark_paid(&owner, bill.id)
            .await
            .unwrap();
        assert_eq!(paid.status, "completed");
        assert_eq!(paid.next_due_at, "2026-02-28 00:00:00");

        // M4: paying a completed bill again must fail
        let second = services.recurring_bills.mark_paid(&owner, bill.id).await;
        assert!(second.is_err());

        // H9: updating currency to JPY updates currency_exponent to 0 and rescales 15.00 -> 15
        let (updated, _) = services
            .recurring_bills
            .update(
                &owner,
                bill.id,
                crate::domain::recurring_bills::RecurringBillPatch {
                    currency: Some("JPY".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.currency, "JPY");
        assert_eq!(updated.currency_exponent, 0);
        assert_eq!(updated.amount_minor, 15);
    }
}
