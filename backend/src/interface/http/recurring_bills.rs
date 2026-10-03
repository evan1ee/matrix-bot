use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::domain::reminders::MAX_LEAD_MINUTES;
use crate::interface::http::auth::{ApiState, AuthUser};
use crate::interface::http::error::ApiError;

/// Represents a recurring bill, subscription, installment, or recurring transfer.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RecurringBill {
    pub id: i64,
    pub room_id: String,
    pub owner_id: String,
    pub sender_name: String,
    pub name: String,
    pub category: String,
    pub amount_minor: i64,
    pub currency: String,
    pub currency_exponent: i64,
    pub frequency: String,
    pub due_day: Option<i32>,
    pub next_due_at: String,
    pub total_installments: Option<i32>,
    pub paid_installments: i32,
    pub auto_pay: bool,
    pub status: String,
    pub notes: Option<String>,
    /// Minutes before `next_due_at` to send a reminder. `None` means no
    /// reminder is wanted.
    pub remind_before_minutes: Option<i64>,
    /// Matrix room the due reminder is delivered to. Defaults to the bill's own
    /// room; required when that is the `web` sentinel.
    pub remind_room_id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

impl From<crate::domain::recurring_bills::RecurringBill> for RecurringBill {
    fn from(b: crate::domain::recurring_bills::RecurringBill) -> Self {
        Self {
            id: b.id,
            room_id: b.room_id,
            owner_id: b.owner_id.to_string(),
            sender_name: b.sender_name,
            name: b.name,
            category: b.category,
            amount_minor: b.amount_minor,
            currency: b.currency,
            currency_exponent: b.currency_exponent,
            frequency: b.frequency,
            due_day: b.due_day,
            next_due_at: b.next_due_at,
            total_installments: b.total_installments,
            paid_installments: b.paid_installments,
            auto_pay: b.auto_pay,
            status: b.status,
            notes: b.notes,
            remind_before_minutes: b.remind_before_minutes,
            remind_room_id: b.remind_room_id,
            created_at: b.created_at,
            updated_at: b.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateBillPayload {
    pub name: String,
    pub category: String,
    pub amount_minor: i64,
    pub currency: String,
    pub currency_exponent: Option<i64>,
    pub frequency: Option<String>,
    pub due_day: Option<i32>,
    pub next_due_at: String,
    pub total_installments: Option<i32>,
    pub auto_pay: Option<bool>,
    pub notes: Option<String>,
    /// Minutes before `next_due_at` to send a reminder.
    pub remind_before_minutes: Option<i64>,
    /// Where to deliver that reminder. Required when the bill has no Matrix
    /// room of its own.
    pub remind_room_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct UpdateBillPayload {
    pub name: Option<String>,
    pub category: Option<String>,
    pub amount_minor: Option<i64>,
    pub currency: Option<String>,
    pub frequency: Option<String>,
    pub due_day: Option<i32>,
    pub next_due_at: Option<String>,
    pub total_installments: Option<i32>,
    pub paid_installments: Option<i32>,
    pub auto_pay: Option<bool>,
    pub status: Option<String>,
    pub notes: Option<String>,
    /// Absent leaves the lead time alone; `null` clears it; a value sets it.
    #[serde(default, deserialize_with = "nullable")]
    #[schema(value_type = Option<i64>)]
    pub remind_before_minutes: Option<Option<i64>>,
    pub remind_room_id: Option<String>,
}

/// Distinguishes an explicit JSON `null` from an absent field, so a PATCH-style
/// update can clear the lead time rather than only overwrite it.
fn nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Rejects lead times that are zero, negative, or absurdly far out.
///
/// Mirrors the todo rule: same bound, same wording, because a user meeting both
/// features should not have to learn two answers to the same question.
fn validate_lead(minutes: i64) -> Result<(), ApiError> {
    if minutes <= 0 {
        return Err(ApiError::bad_request(
            "remind_before_minutes must be greater than zero; send null to clear it",
        ));
    }
    if minutes > MAX_LEAD_MINUTES {
        return Err(ApiError::bad_request(format!(
            "remind_before_minutes must be at most {MAX_LEAD_MINUTES} (30 days)"
        )));
    }
    Ok(())
}

/// A bill created from the web has no room of its own, so a reminder needs one
/// named. Rejecting up front beats writing a lead time that silently never
/// fires.
fn validate_reminder_room(
    lead: Option<i64>,
    room_id: &str,
    explicit: Option<&str>,
) -> Result<(), ApiError> {
    if lead.is_none() {
        return Ok(());
    }
    let named = explicit.map(|r| !r.trim().is_empty()).unwrap_or(false);
    if named || room_id != "web" {
        return Ok(());
    }
    Err(ApiError::bad_request(
        "remind_room_id is required for a reminder on a bill that has no Matrix room",
    ))
}

#[utoipa::path(
    get,
    path = "/api/v1/recurring-bills",
    responses(
        (status = 200, description = "Your recurring bills", body = Vec<RecurringBill>),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn list_bills(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<RecurringBill>>, ApiError> {
    let bills = state
        .services
        .recurring_bills
        .list_by_owner(&user.account_id)
        .await?;

    let items = bills.into_iter().map(RecurringBill::from).collect();
    Ok(Json(items))
}

#[utoipa::path(
    post,
    path = "/api/v1/recurring-bills",
    request_body = CreateBillPayload,
    responses(
        (status = 200, description = "Bill created", body = RecurringBill),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn create_bill(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(payload): Json<CreateBillPayload>,
) -> Result<Json<RecurringBill>, ApiError> {
    if payload.name.trim().is_empty() {
        return Err(ApiError::bad_request("Bill name is required"));
    }
    if payload.name.len() > 500 {
        return Err(ApiError::bad_request("Bill name exceeds 500 characters"));
    }
    if payload.amount_minor <= 0 {
        return Err(ApiError::bad_request("Amount must be greater than zero"));
    }

    if let Some(minutes) = payload.remind_before_minutes {
        validate_lead(minutes)?;
    }
    validate_reminder_room(
        payload.remind_before_minutes,
        "web",
        payload.remind_room_id.as_deref(),
    )?;

    let input = crate::domain::recurring_bills::NewRecurringBill {
        room_id: "web".to_string(),
        sender_name: user.display_name,
        name: payload.name,
        category: payload.category,
        amount_minor: payload.amount_minor,
        currency: payload.currency,
        currency_exponent: payload.currency_exponent.unwrap_or(2),
        frequency: payload.frequency.unwrap_or_else(|| "monthly".to_string()),
        due_day: payload.due_day,
        next_due_at: payload.next_due_at,
        total_installments: payload.total_installments,
        auto_pay: payload.auto_pay.unwrap_or(false),
        notes: payload.notes,
        remind_before_minutes: payload.remind_before_minutes,
        remind_room_id: payload.remind_room_id,
    };

    let (bill, _) = state
        .services
        .recurring_bills
        .create(&user.account_id, input)
        .await?;

    Ok(Json(bill.into()))
}

#[utoipa::path(
    get,
    path = "/api/v1/recurring-bills/{id}",
    params(("id" = i64, Path, description = "Recurring bill ID")),
    responses(
        (status = 200, description = "The bill", body = RecurringBill),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse),
        (status = 404, description = "Not found", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn get_bill(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<RecurringBill>, ApiError> {
    let bill = state
        .services
        .recurring_bills
        .find_owned(&user.account_id, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Recurring bill not found"))?;

    Ok(Json(bill.into()))
}

#[utoipa::path(
    put,
    path = "/api/v1/recurring-bills/{id}",
    params(("id" = i64, Path, description = "Recurring bill ID")),
    request_body = UpdateBillPayload,
    responses(
        (status = 200, description = "Bill updated", body = RecurringBill),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse),
        (status = 404, description = "Not found", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn update_bill(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateBillPayload>,
) -> Result<Json<RecurringBill>, ApiError> {
    if let Some(ref name) = payload.name {
        if name.trim().is_empty() {
            return Err(ApiError::bad_request("Bill name cannot be empty"));
        }
        if name.len() > 500 {
            return Err(ApiError::bad_request("Bill name exceeds 500 characters"));
        }
    }

    if let Some(amount) = payload.amount_minor {
        if amount <= 0 {
            return Err(ApiError::bad_request("Amount must be greater than zero"));
        }
    }

    if let Some(Some(minutes)) = payload.remind_before_minutes {
        validate_lead(minutes)?;
    }

    let patch = crate::domain::recurring_bills::RecurringBillPatch {
        name: payload.name,
        category: payload.category,
        amount_minor: payload.amount_minor,
        currency: payload.currency,
        frequency: payload.frequency,
        due_day: payload.due_day,
        next_due_at: payload.next_due_at,
        total_installments: payload.total_installments,
        paid_installments: payload.paid_installments,
        auto_pay: payload.auto_pay,
        status: payload.status,
        notes: payload.notes,
        remind_before_minutes: payload.remind_before_minutes,
        remind_room_id: payload.remind_room_id,
    };

    let (updated, _) = state
        .services
        .recurring_bills
        .update(&user.account_id, id, patch)
        .await?;

    Ok(Json(updated.into()))
}

#[utoipa::path(
    delete,
    path = "/api/v1/recurring-bills/{id}",
    params(("id" = i64, Path, description = "Recurring bill ID")),
    responses(
        (status = 204, description = "Bill deleted"),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse),
        (status = 404, description = "Not found", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn delete_bill(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    state
        .services
        .recurring_bills
        .delete(&user.account_id, id)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/v1/recurring-bills/{id}/pay",
    params(("id" = i64, Path, description = "Recurring bill ID")),
    responses(
        (status = 200, description = "Bill marked paid for the period, advancing next_due_at", body = RecurringBill),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse),
        (status = 404, description = "Not found", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn pay_bill(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<RecurringBill>, ApiError> {
    let (bill, _tx, _) = state
        .services
        .recurring_bills
        .mark_paid(&user.account_id, id)
        .await?;

    Ok(Json(bill.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbPool;
    use crate::domain::reminders::ReminderSource;
    use crate::infrastructure::postgres::repositories::reminders as ReminderRepository;

    fn state_with(pool: DbPool) -> ApiState {
        ApiState::new(
            crate::db::Database { pool },
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        )
    }

    fn due_in(hours: i64) -> String {
        (chrono::Utc::now() + chrono::Duration::hours(hours))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
    }

    fn payload_with(lead: Option<i64>, room: Option<&str>) -> CreateBillPayload {
        CreateBillPayload {
            name: "Spotify Premium".to_string(),
            category: "subscription".to_string(),
            amount_minor: 999,
            currency: "USD".to_string(),
            currency_exponent: Some(2),
            frequency: Some("monthly".to_string()),
            due_day: None,
            next_due_at: due_in(48),
            total_installments: None,
            auto_pay: Some(false),
            notes: None,
            remind_before_minutes: lead,
            remind_room_id: room.map(str::to_string),
        }
    }

    /// The whole point: a bill with a lead time schedules a reminder, paying it
    /// re-arms for the next period, and deleting it takes the reminder with it.
    #[tokio::test]
    async fn test_bill_lead_time_schedules_rearms_and_tears_down() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["acct_alice", "acct_bob"]).await;
        let state = state_with(pool);
        let alice = AuthUser::named_seed("alice", "acct_alice");

        let bill = create_bill(
            State(state.clone()),
            alice.clone(),
            Json(payload_with(Some(1440), Some("!room:a"))),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(bill.remind_before_minutes, Some(1440));

        let scheduled = ReminderRepository::find_pending_for_source(
            &state.db.pool,
            ReminderSource::RecurringBill,
            bill.id,
        )
        .await
        .unwrap()
        .expect("creating a bill with a lead time must schedule a reminder");
        let first_trigger = scheduled.trigger_at;

        // Paying advances next_due_at, so the reminder moves with it.
        let paid = pay_bill(State(state.clone()), alice.clone(), Path(bill.id))
            .await
            .unwrap()
            .0;
        let rearmed = ReminderRepository::find_pending_for_source(
            &state.db.pool,
            ReminderSource::RecurringBill,
            bill.id,
        )
        .await
        .unwrap()
        .expect("a paid recurring bill still has a next period to remind about");
        assert!(
            rearmed.trigger_at > first_trigger,
            "paying must re-arm at the next period, not leave the old reminder"
        );
        assert_ne!(paid.next_due_at, bill.next_due_at);

        // Clearing the lead time cancels it.
        let _ = update_bill(
            State(state.clone()),
            alice.clone(),
            Path(bill.id),
            Json(UpdateBillPayload {
                remind_before_minutes: Some(None),
                ..Default::default()
            }),
        )
        .await
        .unwrap();
        assert!(ReminderRepository::find_pending_for_source(
            &state.db.pool,
            ReminderSource::RecurringBill,
            bill.id
        )
        .await
        .unwrap()
        .is_none());

        // Re-arm, then delete the bill: nothing may point at a row that is gone.
        let _ = update_bill(
            State(state.clone()),
            alice.clone(),
            Path(bill.id),
            Json(UpdateBillPayload {
                remind_before_minutes: Some(Some(60)),
                ..Default::default()
            }),
        )
        .await
        .unwrap();
        delete_bill(State(state.clone()), alice, Path(bill.id))
            .await
            .unwrap();
        assert!(ReminderRepository::find_pending_for_source(
            &state.db.pool,
            ReminderSource::RecurringBill,
            bill.id
        )
        .await
        .unwrap()
        .is_none());
    }

    /// Validation happens before the write, so a rejected lead never leaves a
    /// bill behind carrying a reminder that could not be scheduled.
    #[tokio::test]
    async fn test_lead_time_is_validated_at_the_boundary() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["acct_alice", "acct_bob"]).await;
        let state = state_with(pool);
        let alice = AuthUser::named_seed("alice", "acct_alice");

        for bad in [0, -5, MAX_LEAD_MINUTES + 1] {
            let err = create_bill(
                State(state.clone()),
                alice.clone(),
                Json(payload_with(Some(bad), Some("!room:a"))),
            )
            .await
            .unwrap_err();
            assert_eq!(
                err.status,
                StatusCode::BAD_REQUEST,
                "lead {bad} must be rejected"
            );
        }

        // A web bill has no room of its own, so a reminder needs one named.
        let err = create_bill(
            State(state.clone()),
            alice.clone(),
            Json(payload_with(Some(60), None)),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);

        assert!(
            list_bills(State(state.clone()), alice)
                .await
                .unwrap()
                .0
                .is_empty(),
            "a rejected lead time must not leave a bill behind"
        );
    }

    #[tokio::test]
    async fn test_recurring_bills_api_flow() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["acct_alice", "acct_bob"]).await;
        let state = ApiState::new(
            crate::db::Database { pool },
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );

        let alice = AuthUser::named_seed("alice", "acct_alice");
        let bob = AuthUser::named_seed("bob", "acct_bob");

        // Create
        let req = CreateBillPayload {
            name: "Spotify Premium".to_string(),
            category: "subscription".to_string(),
            amount_minor: 999,
            currency: "USD".to_string(),
            currency_exponent: Some(2),
            frequency: Some("monthly".to_string()),
            due_day: Some(1),
            next_due_at: "2026-09-01 00:00:00".to_string(),
            total_installments: None,
            auto_pay: Some(true),
            notes: None,
            remind_before_minutes: None,
            remind_room_id: None,
        };

        let bill = create_bill(State(state.clone()), alice.clone(), Json(req))
            .await
            .unwrap()
            .0;

        assert_eq!(bill.name, "Spotify Premium");
        assert_eq!(bill.amount_minor, 999);

        // Bob cannot see Alice's bill
        let bob_bills = list_bills(State(state.clone()), bob.clone())
            .await
            .unwrap()
            .0;
        assert!(bob_bills.is_empty());

        // Mark paid
        let paid = pay_bill(State(state.clone()), alice.clone(), Path(bill.id))
            .await
            .unwrap()
            .0;

        assert_eq!(paid.paid_installments, 1);
        assert_eq!(paid.next_due_at, "2026-10-01 00:00:00");

        // Verify bill payment automatically posted an expense transaction to the finance ledger
        let txs = state
            .services
            .finance
            .list_transactions(
                &alice.account_id,
                &crate::domain::finance::ListTransactionsFilter {
                    limit: Some(10),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].amount_minor, 999);
        assert_eq!(txs[0].direction, crate::domain::finance::Direction::Expense);
        assert_eq!(txs[0].source_kind.as_deref(), Some("recurring_bill"));
        assert_eq!(txs[0].source_id, Some(bill.id));
        assert_eq!(txs[0].category_name.as_deref(), Some("subscription"));

        // Delete
        let status = delete_bill(State(state.clone()), alice, Path(bill.id))
            .await
            .unwrap();
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
}
