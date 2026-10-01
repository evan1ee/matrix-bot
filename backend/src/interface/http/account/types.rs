//! Request and response shapes shared by the account endpoints.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// How many rows each table would contribute to a merge.
///
/// Shown to the user before they confirm, because "this will move 47 todos and
/// 132 transactions into your account" is a decision someone should get to
/// make with the numbers in front of them.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct MergePreview {
    pub todos: i64,
    pub reminders: i64,
    pub transactions: i64,
    pub recurring_bills: i64,
    pub portfolio_holdings: i64,
    pub portfolio_cash_balances: i64,
    pub finance_categories: i64,
    pub user_settings: i64,
    pub bookmarks: i64,
    pub habits: i64,
    pub habit_logs: i64,
    pub life_day_logs: i64,
    pub life_profiles: i64,
    pub footprints: i64,
    pub ai_conversations: i64,
    pub webhooks: i64,
    pub other: i64,
}

impl From<crate::domain::accounts::MergePreview> for MergePreview {
    fn from(p: crate::domain::accounts::MergePreview) -> Self {
        Self {
            todos: p.todos,
            reminders: p.reminders,
            transactions: p.transactions,
            recurring_bills: p.recurring_bills,
            portfolio_holdings: p.portfolio_holdings,
            portfolio_cash_balances: p.portfolio_cash_balances,
            finance_categories: p.finance_categories,
            user_settings: p.user_settings,
            bookmarks: p.bookmarks,
            habits: p.habits,
            habit_logs: p.habit_logs,
            life_day_logs: p.life_day_logs,
            life_profiles: p.life_profiles,
            footprints: p.footprints,
            ai_conversations: p.ai_conversations,
            webhooks: p.webhooks,
            other: p.other,
        }
    }
}

impl From<MergePreview> for crate::domain::accounts::MergePreview {
    fn from(p: MergePreview) -> Self {
        Self {
            todos: p.todos,
            reminders: p.reminders,
            transactions: p.transactions,
            recurring_bills: p.recurring_bills,
            portfolio_holdings: p.portfolio_holdings,
            portfolio_cash_balances: p.portfolio_cash_balances,
            finance_categories: p.finance_categories,
            user_settings: p.user_settings,
            bookmarks: p.bookmarks,
            habits: p.habits,
            habit_logs: p.habit_logs,
            life_day_logs: p.life_day_logs,
            life_profiles: p.life_profiles,
            footprints: p.footprints,
            ai_conversations: p.ai_conversations,
            webhooks: p.webhooks,
            other: p.other,
        }
    }
}
#[derive(Debug, Serialize, ToSchema)]
pub struct PasskeySummary {
    pub id: String,
    pub label: String,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SessionSummary {
    /// The session's storage key. Safe to expose — it is a hash, not the
    /// cookie, so it cannot be replayed.
    pub id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_seen_at: chrono::DateTime<chrono::Utc>,
    pub user_agent: Option<String>,
    /// Whether this is the session making the request.
    pub current: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UnlinkRequest {
    pub mxid: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecoveryCodesResponse {
    /// Shown exactly once. The server keeps only hashes.
    pub codes: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RenamePasskeyRequest {
    pub label: String,
}

// One row of the account's security log, as the web UI reads it.
//
// The domain event also carries the owning account; the wire shape does not,
// because the caller can only ever list their own.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SecurityEvent {
    pub id: i64,
    pub kind: String,
    pub detail: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

impl From<&crate::domain::accounts::SecurityEvent> for SecurityEvent {
    fn from(ev: &crate::domain::accounts::SecurityEvent) -> Self {
        Self {
            id: ev.id,
            kind: ev.kind.clone(),
            detail: ev.detail.clone(),
            created_at: ev.created_at,
        }
    }
}
