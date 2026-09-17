use axum::{response::IntoResponse, Json};
use utoipa::openapi::security::SecurityScheme;
use utoipa::{Modify, OpenApi};

use crate::api::{
    account, ai, auth, bookmarks, briefing, finance, finance_analytics, finance_net_worth, health, portfolio, portfolio_alerts,
    portfolio_cash, portfolio_history, portfolio_prices, recurring_bills, reminders, rooms,
    settings, sse, todos, webhooks,
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
use crate::modules::settings::UserSettings;
use crate::modules::todo::models::{TodoItem, TodoPriority};

#[derive(OpenApi)]
#[openapi(
    paths(
        health::health_check,
        sse::sse_handler,
        auth::webauthn::enroll_start,
        auth::webauthn::register_start,
        auth::webauthn::register_finish,
        auth::webauthn::login_start,
        auth::webauthn::login_finish,
        auth::matrix::login_with_matrix,
        auth::logout,
        account::passkeys::list_passkeys,
        account::passkeys::add_passkey_start,
        account::passkeys::rename_passkey,
        account::passkeys::delete_passkey,
        account::links::list_links,
        account::links::start_link,
        account::links::pending_merge,
        account::links::confirm_merge,
        account::links::unlink,
        account::sessions::list_sessions,
        account::sessions::revoke_other_sessions,
        account::sessions::revoke_session,
        account::recovery::regenerate_recovery_codes,
        account::recovery::recovery_status,
        account::recovery::cancel_recovery,
        account::audit::list_security_events,
        auth::get_current_user,
        auth::media_proxy,
        rooms::list_rooms,
        rooms::sync_rooms,
        settings::get_settings,
        settings::update_settings,
        briefing::trigger_briefing,
        todos::list_todos,
        todos::create_todo,
        todos::update_todo,
        todos::delete_todo,
        reminders::list_reminders,
        reminders::create_reminder,
        reminders::update_reminder,
        reminders::delete_reminder,
        reminders::clear_reminder_history,
        finance::list_transactions,
        finance::create_transaction,
        finance::update_transaction,
        finance::delete_transaction,
        finance::summary,
        finance_net_worth::get_net_worth,
        finance::list_categories,
        finance::create_category,
        finance::delete_category,
        finance_analytics::get_analytics,
        finance_analytics::get_share,
        finance_analytics::upsert_share,
        finance_analytics::delete_share,
        finance_analytics::get_public_report,
        portfolio::list_holdings,
        portfolio::buy_holding,
        portfolio::sell_holding,
        portfolio::set_price,
        portfolio::set_note,
        portfolio::delete_holding,
        portfolio::summary,
        portfolio_cash::list_cash,
        portfolio_cash::deposit_cash,
        portfolio_cash::withdraw_cash,
        portfolio_prices::refresh_all_prices,
        portfolio_prices::refresh_single_holding_price,
        portfolio_prices::record_holding_position,
        portfolio_prices::get_portfolio_activity,
        portfolio_prices::verify_portfolio_activity_chain,
        portfolio_prices::search_ticker_symbols,
        portfolio_prices::get_symbol_quote,
        portfolio_prices::update_holding_symbol,
        portfolio_alerts::list_alerts,
        portfolio_alerts::create_alert,
        portfolio_alerts::delete_alert,
        portfolio_history::get_portfolio_history,
        recurring_bills::list_bills,
        recurring_bills::create_bill,
        recurring_bills::get_bill,
        recurring_bills::update_bill,
        recurring_bills::delete_bill,
        recurring_bills::pay_bill,
        webhooks::inbound_webhook,
        webhooks::list_webhooks,
        webhooks::create_webhook,
        webhooks::update_webhook,
        webhooks::rotate_webhook_token,
        webhooks::test_webhook,
        webhooks::delete_webhook,
        webhooks::get_webhook_logs,
        ai::chat,
        ai::verify_ai_config,
        ai::list_conversations,
        ai::get_conversation_messages,
        ai::delete_conversation,
        ai::get_ai_config,
        ai::update_ai_config,
        ai::get_retrospective,
        bookmarks::list_bookmarks,
        bookmarks::create_bookmark,
        bookmarks::get_stats,
        bookmarks::get_bookmark,
        bookmarks::update_bookmark,
        bookmarks::toggle_favorite,
        bookmarks::toggle_archive,
        bookmarks::delete_bookmark
    ),
    components(
        schemas(
            ApiErrorResponse,
            health::HealthResponse,
            auth::webauthn::CeremonyStart,
            auth::webauthn::RegisterStartRequest,
            auth::webauthn::EnrollStartRequest,
            auth::webauthn::RegisterFinishRequest,
            auth::webauthn::LoginFinishRequest,
            auth::webauthn::SessionResponse,
            auth::matrix::MatrixLoginRequest,
            auth::matrix::MatrixLoginResponse,
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
            crate::modules::briefing::models::BriefingPayload,
            crate::modules::briefing::models::BriefingSummary,
            crate::modules::briefing::models::TickerPerformance,
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
            crate::modules::finance::net_worth::NetWorthResponse,
            crate::modules::finance::net_worth::CurrencyNetWorthBreakdown,
            crate::modules::finance::analytics::MonthlyCashflowBucket,
            crate::modules::finance::analytics::ExpenseCategorySlice,
            crate::modules::finance::analytics::FinanceAnalyticsReport,
            crate::modules::finance::analytics::FinanceShare,
            crate::modules::finance::analytics::PublicTransaction,
            crate::modules::finance::analytics::PublicFinanceReport,
            finance_analytics::AnalyticsQuery,
            finance_analytics::PublicQuery,
            finance_analytics::ShareRequest,
            finance_analytics::StatusResponse,
            Holding,
            PortfolioCurrencySummary,
            CashBalance,
            portfolio::BuyHoldingRequest,
            portfolio::SellHoldingRequest,
            portfolio::SetPriceRequest,
            portfolio::SetNoteRequest,
            portfolio::SellResponse,
            portfolio_cash::CashAmountRequest,
            crate::modules::portfolio::price_feed::RefreshSummary,
            crate::modules::portfolio::price_feed::TickerSearchResult,
            portfolio_prices::UpdateSymbolRequest,
            portfolio_prices::SymbolQuoteResponse,
            portfolio_alerts::CreateAlertRequest,
            crate::modules::portfolio::alerts::PriceAlert,
            crate::modules::portfolio::alerts::AlertCondition,
            crate::modules::portfolio::history::PortfolioHistoryResponse,
            crate::modules::portfolio::history::PortfolioHistoryPoint,
            crate::modules::portfolio::audit::PortfolioAuditEntry,
            crate::modules::portfolio::audit::AuditVerificationResult,
            RecurringBill,
            CreateBillPayload,
            UpdateBillPayload,
            crate::modules::webhook::models::Webhook,
            crate::modules::webhook::models::WebhookOutboxItem,
            crate::modules::webhook::models::WebhookSeverity,
            crate::modules::webhook::models::CreatedWebhookResponse,
            crate::modules::webhook::models::CreateWebhookInput,
            crate::modules::webhook::models::UpdateWebhookInput,
            crate::modules::webhook::models::InboundPayload,
            crate::modules::webhook::models::InboundAcceptedResponse,
            crate::modules::webhook::models::WebhookLogsResponse,
            crate::modules::ai::models::AiUserConfig,
            crate::modules::ai::models::UpdateAiConfigInput,
            crate::modules::ai::models::VerifyAiKeyInput,
            crate::modules::ai::models::VerifyAiKeyResponse,
            crate::modules::ai::models::AiConversation,
            crate::modules::ai::models::AiMessage,
            crate::modules::ai::models::AiChatRequest,
            crate::modules::ai::models::AiChatResponse,
            crate::modules::ai::retrospective::WeeklyRetrospective,
            crate::modules::ai::retrospective::CurrencySpendSummary,
            crate::modules::ai::retrospective::UpcomingBillsSummary,
            crate::modules::bookmarks::models::Bookmark,
            crate::modules::bookmarks::models::CreateBookmarkInput,
            crate::modules::bookmarks::models::UpdateBookmarkInput,
            crate::modules::bookmarks::models::BookmarkStats,
            crate::modules::bookmarks::models::DomainCount,
            crate::modules::mcp::models::CreateMcpTokenRequest,
            crate::modules::mcp::models::CreateMcpTokenResponse,
            crate::modules::mcp::models::McpTokenSummary
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
        components.add_security_scheme(
            "bearer_auth",
            SecurityScheme::Http(utoipa::openapi::security::Http::new(
                utoipa::openapi::security::HttpAuthScheme::Bearer,
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
