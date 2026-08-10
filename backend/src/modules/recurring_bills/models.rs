use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// Represents a recurring bill, subscription, installment, or recurring transfer.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
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
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}
