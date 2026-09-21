use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::events::SystemEvent;
use crate::modules::due_reminder::MAX_LEAD_MINUTES;
use crate::modules::recurring_bills::due_reminder;
use crate::modules::recurring_bills::models::{
    CreateBillPayload, RecurringBill, UpdateBillPayload,
};
use crate::modules::recurring_bills::repository::RecurringBillRepository;

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
    let owner_id = user.owner_id();
    let bills = RecurringBillRepository::list_by_owner(&state.db.pool, &owner_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(bills))
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

    let owner_id = user.owner_id();
    let display_name = user.display_name;

    let bill =
        RecurringBillRepository::create(&state.db.pool, "web", &owner_id, &display_name, payload)
            .await
            .map_err(|e| ApiError::bad_request(e.to_string()))?;

    due_reminder::sync(&state.db.pool, Some(&state.broadcaster), &bill)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    state
        .broadcaster
        .send(SystemEvent::RecurringBillCreated(bill.clone()));

    Ok(Json(bill))
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
    let owner_id = user.owner_id();
    let bill = RecurringBillRepository::get_by_id(&state.db.pool, id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Recurring bill not found"))?;

    if bill.owner_id != owner_id {
        return Err(ApiError::not_found("Recurring bill not found"));
    }

    Ok(Json(bill))
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
    let owner_id = user.owner_id();

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

    let updated = RecurringBillRepository::update(&state.db.pool, id, &owner_id, payload)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => {
                ApiError::not_found("Recurring bill not found or access denied")
            }
            err => ApiError::internal(err.to_string()),
        })?;

    // Reconcile from the stored state, so a changed due date, lead time, or
    // status reschedules or cancels the reminder without a per-field rule here.
    due_reminder::sync(&state.db.pool, Some(&state.broadcaster), &updated)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    state
        .broadcaster
        .send(SystemEvent::RecurringBillUpdated(updated.clone()));

    Ok(Json(updated))
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
    let owner_id = user.owner_id();
    let success = RecurringBillRepository::delete(&state.db.pool, id, &owner_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    if !success {
        return Err(ApiError::not_found(
            "Recurring bill not found or access denied",
        ));
    }

    // The bill is gone; its reminder must not outlive it pointing at nothing.
    due_reminder::tear_down(&state.db.pool, Some(&state.broadcaster), id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    state
        .broadcaster
        .send(SystemEvent::RecurringBillDeleted { id, owner_id });

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
    let owner_id = user.owner_id();
    // The finance service broadcasts `TransactionCreated` itself, so paying a
    // bill announces the resulting transaction the same way any other write
    // through the service does — this call site only announces the parts
    // specific to the bill.
    let (bill, _tx) =
        RecurringBillRepository::mark_paid(&state.db.pool, id, &owner_id, Some(&state.broadcaster))
            .await
            .map_err(|e| match e {
                sqlx::Error::RowNotFound => {
                    ApiError::not_found("Recurring bill not found or access denied")
                }
                err => ApiError::internal(err.to_string()),
            })?;

    // Paying advanced `next_due_at`, so re-arm at the next period — or tear
    // down if that was the final installment.
    due_reminder::sync(&state.db.pool, Some(&state.broadcaster), &bill)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    state.broadcaster.send(SystemEvent::RecurringBillPaid {
        id: bill.id,
        owner_id: owner_id.clone(),
        next_due_at: bill.next_due_at.clone(),
    });
    state
        .broadcaster
        .send(SystemEvent::RecurringBillUpdated(bill.clone()));

    Ok(Json(bill))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::db::DbPool;
    use crate::events::EventBroadcaster;
    use crate::modules::reminder::models::ReminderSource;
    use crate::modules::reminder::repository::ReminderRepository;

    fn state_with(pool: DbPool) -> ApiState {
        ApiState::new(
            crate::db::Database { pool: pool },
            EventBroadcaster::default(),
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
        run_migrations(&pool).await.unwrap();
        let state = state_with(pool);
        let alice = AuthUser::named("alice", "acct_alice");

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
        run_migrations(&pool).await.unwrap();
        let state = state_with(pool);
        let alice = AuthUser::named("alice", "acct_alice");

        for bad in [0, -5, crate::modules::due_reminder::MAX_LEAD_MINUTES + 1] {
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
        run_migrations(&pool).await.unwrap();
        let state = ApiState::new(
            crate::db::Database { pool: pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );

        let alice = AuthUser::named("alice", "acct_alice");
        let bob = AuthUser::named("bob", "acct_bob");

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
        let txs = crate::modules::finance::repository::FinanceRepository::list(
            &state.db.pool,
            &alice.owner_id(),
            None,
            10,
            0,
        )
        .await
        .unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].amount_minor, 999);
        assert_eq!(txs[0].direction, "expense");
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
