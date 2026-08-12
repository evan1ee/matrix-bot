use axum::{response::IntoResponse, Json};
use utoipa::openapi::security::SecurityScheme;
use utoipa::{Modify, OpenApi};

use crate::api::{
    account, auth, finance, health, portfolio, recurring_bills, reminders, rooms, settings, sse,
    todos,
};
use crate::error::ApiErrorResponse;
use crate::modules::finance::models::{
    Category, CategoryTotal, CurrencySummary, Direction, Transaction,
};
use crate::modules::portfolio::models::{CashBalance, Holding, PortfolioCurrencySummary};
use crate::modules::recurring_bills::models::{
    CreateBillPayload, RecurringBill, UpdateBillPayload,
};
use crate::modules::reminder::models::{RecurrencePattern, Reminder, ReminderStatus};
use crate::modules::todo::models::{TodoItem, TodoPriority};
use crate::modules::settings::UserSettings;

#[derive(OpenApi)]
#[openapi(
    paths(
        health::health_check,
        sse::sse_handler,
        auth::webauthn::register_start,
        auth::webauthn::register_finish,
        auth::webauthn::login_start,
        auth::webauthn::login_finish,
        auth::logout,
        account::passkeys::list_passkeys,
        account::passkeys::rename_passkey,
        account::passkeys::delete_passkey,
        account::links::list_links,
        account::links::start_link,
        account::links::pending_merge,
        account::links::confirm_merge,
        account::links::unlink,
        account::sessions::list_sessions,
        account::sessions::revoke_other_sessions,
        account::recovery::regenerate_recovery_codes,
        account::recovery::recovery_status,
        account::recovery::cancel_recovery,
        account::audit::list_security_events,
        auth::get_current_user,
        auth::media_proxy,
        rooms::list_rooms,
        settings::get_settings,
        settings::update_settings,
        todos::list_todos,
        todos::create_todo,
        todos::update_todo,
        todos::delete_todo,
        reminders::list_reminders,
        reminders::create_reminder,
        reminders::update_reminder,
        reminders::delete_reminder,
        finance::list_transactions,
        finance::create_transaction,
        finance::update_transaction,
        finance::delete_transaction,
        finance::summary,
        finance::list_categories,
        finance::create_category,
        finance::delete_category,
        portfolio::list_holdings,
        portfolio::buy_holding,
        portfolio::sell_holding,
        portfolio::set_price,
        portfolio::set_note,
        portfolio::delete_holding,
        portfolio::summary,
        portfolio::list_cash,
        portfolio::deposit_cash,
        portfolio::withdraw_cash,
        recurring_bills::list_bills,
        recurring_bills::create_bill,
        recurring_bills::get_bill,
        recurring_bills::update_bill,
        recurring_bills::delete_bill,
        recurring_bills::pay_bill
    ),
    components(
        schemas(
            ApiErrorResponse,
            health::HealthResponse,
            auth::webauthn::CeremonyStart,
            auth::webauthn::RegisterStartRequest,
            auth::webauthn::RegisterFinishRequest,
            auth::webauthn::LoginFinishRequest,
            auth::webauthn::SessionResponse,
            account::PasskeySummary,
            account::SessionSummary,
            account::LinkCodeResponse,
            account::UnlinkRequest,
            account::PendingMerge,
            account::ConfirmMergeRequest,
            account::RecoveryCodesResponse,
            account::RenamePasskeyRequest,
            crate::accounts::MergePreview,
            crate::accounts::security_events::SecurityEvent,
            auth::UserProfileResponse,
            auth::MediaProxyQuery,
            rooms::JoinedRoomInfo,
            UserSettings,
            settings::UpdateSettingsRequest,
            TodoItem,
            TodoPriority,
            todos::TodoQuery,
            todos::CreateTodoRequest,
            todos::UpdateTodoRequest,
            todos::TodoListResponse,
            Reminder,
            ReminderStatus,
            RecurrencePattern,
            reminders::ReminderQuery,
            reminders::CreateReminderRequest,
            reminders::UpdateReminderRequest,
            reminders::ReminderListResponse,
            Transaction,
            Category,
            Direction,
            CategoryTotal,
            CurrencySummary,
            finance::LedgerQuery,
            finance::SummaryQuery,
            finance::TransactionRequest,
            finance::CategoryRequest,
            finance::LedgerResponse,
            finance::CategoryDeletedResponse,
            Holding,
            PortfolioCurrencySummary,
            CashBalance,
            portfolio::BuyHoldingRequest,
            portfolio::SellHoldingRequest,
            portfolio::SetPriceRequest,
            portfolio::SetNoteRequest,
            portfolio::SellResponse,
            portfolio::CashAmountRequest,
            RecurringBill,
            CreateBillPayload,
            UpdateBillPayload
        )
    ),
    modifiers(&SecurityAddon),
    tags(
        (name = "Matrix Bot REST API", description = "REST API & Real-time Web interface endpoints")
    )
)]
pub struct ApiDoc;

/// Declares the `session_cookie` scheme every secured path references.
///
/// Without this the generated document carries security requirements pointing
/// at a scheme that was never declared, which generated clients cannot wire up.
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi
            .components
            .get_or_insert_with(utoipa::openapi::Components::default);

        components.add_security_scheme(
            "session_cookie",
            SecurityScheme::ApiKey(utoipa::openapi::security::ApiKey::Cookie(
                utoipa::openapi::security::ApiKeyValue::with_description(
                    crate::accounts::session::COOKIE_NAME,
                    "Session cookie set by the passkey sign-in endpoints. HttpOnly, so \
                     it is sent by the browser automatically and is not readable by \
                     JavaScript.",
                ),
            )),
        );
    }
}

/// Serves the generated document.
pub async fn openapi_json() -> impl IntoResponse {
    Json(ApiDoc::openapi())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The document must build and declare every scheme its paths reference.
    ///
    /// Writing `openapi.json` is the job of the `generate-openapi` binary, not of
    /// this test — a test that writes files drifts silently when the suite runs
    /// from a different directory.
    #[test]
    fn test_openapi_document_is_well_formed() {
        let doc = ApiDoc::openapi();
        let json = doc.to_pretty_json().unwrap();
        assert!(!json.is_empty());

        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let schemes = &parsed["components"]["securitySchemes"];
        assert!(
            schemes["session_cookie"].is_object(),
            "paths reference `bearer_auth`, so it must be declared"
        );
    }
}
