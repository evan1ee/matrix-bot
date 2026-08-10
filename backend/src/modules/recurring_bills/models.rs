use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;

/// Represents a recurring bill, subscription, installment, or recurring transfer.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct RecurringBill {
    pub id: i64,
    pub room_id: String,
    pub sender_mxid: String,
    pub sender_name: String,
    pub name: String,
    pub category: String,
    pub amount_minor: i64,
    pub currency: String,
    pub currency_exponent: i32,
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateBillPayload {
    pub name: String,
    pub category: String,
    pub amount_minor: i64,
    pub currency: String,
    pub currency_exponent: Option<i32>,
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
