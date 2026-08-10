use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::events::SystemEvent;
use crate::modules::recurring_bills::models::{
    CreateBillPayload, RecurringBill, UpdateBillPayload,
};
use crate::modules::recurring_bills::repository::RecurringBillRepository;

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

    let owner_id = user.owner_id();
    let display_name = user.display_name();

    let bill = RecurringBillRepository::create(
        &state.db.pool,
        "web",
        &owner_id,
        &display_name,
        payload,
    )
    .await
    .map_err(|e| ApiError::bad_request(e.to_string()))?;

    state.broadcaster.send(SystemEvent::RecurringBillCreated(bill.clone()));

    Ok(Json(bill))
}

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

    if bill.sender_mxid != owner_id {
        return Err(ApiError::not_found("Recurring bill not found"));
    }

    Ok(Json(bill))
}

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

    let updated = RecurringBillRepository::update(&state.db.pool, id, &owner_id, payload)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => ApiError::not_found("Recurring bill not found or access denied"),
            err => ApiError::internal(err.to_string()),
        })?;

    state.broadcaster.send(SystemEvent::RecurringBillUpdated(updated.clone()));

    Ok(Json(updated))
}

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
        return Err(ApiError::not_found("Recurring bill not found or access denied"));
    }

    state.broadcaster.send(SystemEvent::RecurringBillDeleted {
        id,
        owner_id,
    });

    Ok(StatusCode::NO_CONTENT)
}

pub async fn pay_bill(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<RecurringBill>, ApiError> {
    let owner_id = user.owner_id();
    let bill = RecurringBillRepository::mark_paid(&state.db.pool, id, &owner_id)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => ApiError::not_found("Recurring bill not found or access denied"),
            err => ApiError::internal(err.to_string()),
        })?;

    state.broadcaster.send(SystemEvent::RecurringBillPaid {
        id: bill.id,
        owner_id: owner_id.clone(),
        next_due_at: bill.next_due_at.clone(),
    });
    state.broadcaster.send(SystemEvent::RecurringBillUpdated(bill.clone()));

    Ok(Json(bill))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::events::EventBroadcaster;
    use sqlx::SqlitePool;

    #[tokio::test]
    async fn test_recurring_bills_api_flow() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let state = ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            "secret".to_string(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );

        let alice = AuthUser::local("alice");
        let bob = AuthUser::local("bob");

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

        // Delete
        let status = delete_bill(State(state.clone()), alice, Path(bill.id))
            .await
            .unwrap();
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
}
