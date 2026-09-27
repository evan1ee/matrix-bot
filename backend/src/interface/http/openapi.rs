use axum::{response::IntoResponse, Json};
use utoipa::openapi::security::SecurityScheme;
use utoipa::{Modify, OpenApi};

use crate::error::ApiErrorResponse;
use crate::interface::http::{
    account, ai, auth, bookmarks, briefing, finance, finance_analytics, finance_net_worth,
    footprints, habits, health, life, life_activity, mcp, ocr, portfolio, portfolio_alerts,
    portfolio_cash, portfolio_history, portfolio_prices, recurring_bills, reminders, rooms,
    settings, sse, todos, voice, webhooks, webhooks_telegram,
};
use crate::modules::finance::models::{
    Category, CategoryTotal, CurrencySummary, Direction, Transaction,
};
use crate::modules::footprints::models::{
    CreateFootprintRequest, Footprint, FootprintCurrencySpend, FootprintsSummary,
    UpdateFootprintRequest,
};
use crate::modules::habits::models::{
    DayStatus, Habit, HabitStatsSummary, HabitWithStatus, HeatmapDay,
};
use crate::modules::life::activity::{ActivityEvent, ActivityStats, DayActivityResponse};
use crate::modules::life::models::{
    LifeDayLog, LifeHorizonDay, LifeProfile, LifeProfileStats, LifeYearView,
    UpdateTimeAllocationInput, UpsertLifeDayLogInput, UpsertLifeProfileInput,
};
use crate::modules::portfolio::models::{CashBalance, Holding, PortfolioCurrencySummary};
use crate::modules::recurring_bills::models::{
    CreateBillPayload, RecurringBill, UpdateBillPayload,
};
use crate::modules::reminder::models::{RecurrencePattern, Reminder, ReminderStatus};
use crate::modules::todo::models::{TodoItem, TodoPriority};
use crate::modules::webhook::models::{TelegramMessageResult, TelegramQuery, TelegramResponse};

#[derive(OpenApi)]
#[openapi(
    paths(
        health::live,
        health::ready,
        sse::sse_handler,
        auth::webauthn::enroll_start,
        auth::webauthn::register_start,
        auth::webauthn::register_finish,
        auth::webauthn::login_start,
        auth::webauthn::login_finish,
        auth::matrix::login_with_matrix,
        auth::recovery::redeem_recovery,
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
        bookmarks::delete_bookmark,
        mcp::list_mcp_tokens,
        mcp::create_mcp_token,
        mcp::delete_mcp_token,
        ocr::scan_image,
        voice::transcribe_voice,
        footprints::list_footprints,
        footprints::create_footprint,
        footprints::get_footprint,
        footprints::update_footprint,
        footprints::delete_footprint,
        footprints::get_summary_stats,
        footprints::geocode_proxy,
        habits::list_habits,
        habits::create_habit,
        habits::update_habit,
        habits::delete_habit,
        habits::toggle_habit_log,
        habits::get_heatmap,
        habits::get_stats,
        life::get_profile_and_stats,
        life::upsert_profile,
        life::get_year_view,
        life::get_horizon,
        life::upsert_day,
        life::update_time_allocation,
        life::get_day,
        life::delete_day,
        life_activity::get_day_activity,
        webhooks_telegram::telegram_send_message_get,
        webhooks_telegram::telegram_send_message_post
    ),
    components(
        schemas(
            ApiErrorResponse,
            health::LiveResponse,
            health::ReadyResponse,
            health::ReadyChecks,
            health::DatabaseCheck,
            health::MatrixSyncCheck,
            auth::webauthn::CeremonyStart,
            auth::webauthn::RegisterStartRequest,
            auth::webauthn::EnrollStartRequest,
            auth::webauthn::RegisterFinishRequest,
            auth::webauthn::LoginFinishRequest,
            auth::webauthn::SessionResponse,
            auth::matrix::MatrixLoginRequest,
            auth::matrix::MatrixLoginResponse,
            auth::recovery::RedeemRecoveryRequest,
            account::PasskeySummary,
            account::SessionSummary,
            account::LinkCodeResponse,
            account::UnlinkRequest,
            account::PendingMerge,
            account::ConfirmMergeRequest,
            account::RecoveryCodesResponse,
            account::RenamePasskeyRequest,
            account::MergePreview,
            crate::accounts::security_events::SecurityEvent,
            auth::UserProfileResponse,
            auth::MediaProxyQuery,
            rooms::JoinedRoomInfo,
            settings::UserSettings,
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
            bookmarks::Bookmark,
            bookmarks::CreateBookmarkInput,
            bookmarks::UpdateBookmarkInput,
            bookmarks::BookmarkStats,
            bookmarks::DomainCount,
            crate::modules::mcp::models::CreateMcpTokenRequest,
            crate::modules::mcp::models::CreateMcpTokenResponse,
            crate::modules::mcp::models::McpTokenSummary,
            ocr::OcrScanResponse,
            ocr::OcrBlockDto,
            ocr::ReceiptExtractionDto,
            voice::VoiceTranscribeResponse,
            Footprint,
            CreateFootprintRequest,
            UpdateFootprintRequest,
            FootprintsSummary,
            FootprintCurrencySpend,
            Habit,
            HabitWithStatus,
            DayStatus,
            HeatmapDay,
            HabitStatsSummary,
            habits::CreateHabitRequest,
            habits::UpdateHabitRequest,
            habits::ToggleHabitRequest,
            LifeProfile,
            LifeDayLog,
            LifeHorizonDay,
            LifeProfileStats,
            LifeYearView,
            UpsertLifeProfileInput,
            UpsertLifeDayLogInput,
            UpdateTimeAllocationInput,
            life::DeleteDayResponse,
            ActivityEvent,
            ActivityStats,
            DayActivityResponse,
            TelegramQuery,
            TelegramResponse,
            TelegramMessageResult
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

    /// Every route the server serves must appear in the document CI diffs.
    ///
    /// Six API files carried no annotations at all, so their endpoints were
    /// absent from `openapi.json` and from the generated frontend client, and
    /// the CI check that diffs that document could not see them. Routes are
    /// read out of `routes.rs` rather than listed here, because a hand-kept
    /// second copy is the thing that drifts.
    #[test]
    fn test_every_route_is_documented() {
        // Routes that genuinely should not be in the document, each with why.
        const UNDOCUMENTED: &[&str] = &[
            // Serves the OpenAPI document itself; documenting the endpoint
            // that serves the documentation would be circular.
            "/api/v1/api-docs/openapi.json",
            // MCP SSE transport handshake: an `event-stream` response with no
            // JSON request or response body to describe.
            "/api/v1/mcp/sse",
            // MCP JSON-RPC messages (SSE session mode): a generic JSON-RPC
            // 2.0 envelope whose real shape depends on the method being
            // called, so one schema would misdescribe every method but one.
            "/api/v1/mcp/messages",
            // MCP direct JSON-RPC endpoint: same reasoning as `/mcp/messages`.
            "/api/v1/mcp/",
            // This handler is nested here too (`routes_ai_and_webhooks`
            // mounts it under `/api/v1`), but it is documented at the one
            // path it is *meant* to be reached at: `/bot{token}/sendMessage`,
            // mounted outside `/api/v1` in `interface/http/mod.rs` to mirror
            // Telegram's own unprefixed Bot API URL shape. The `/api/v1`-nested
            // mount is an incidental duplicate of that same handler, not a
            // second documented surface.
            "/api/v1/bot{token}/sendMessage",
        ];

        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

        let routes_src = std::fs::read_to_string(manifest_dir.join("src/interface/http/routes.rs"))
            .expect("src/interface/http/routes.rs must be readable");
        let mut routes: Vec<String> = extract_route_paths(&routes_src)
            .iter()
            .map(|path| format!("/api/v1{}", axum_path_to_openapi(path)))
            .collect();

        // `.nest("/mcp", mcp::mcp_routes())` mounts a whole other route table
        // defined in `mcp.rs`; `routes.rs` only names the mount point, not
        // the routes underneath it, so that file is parsed separately.
        let mcp_src = std::fs::read_to_string(manifest_dir.join("src/interface/http/mcp.rs"))
            .expect("src/interface/http/mcp.rs must be readable");
        routes.extend(
            extract_route_paths(&mcp_src)
                .iter()
                .map(|path| format!("/api/v1/mcp{}", axum_path_to_openapi(path))),
        );

        let doc = ApiDoc::openapi();
        let documented: std::collections::HashSet<&str> =
            doc.paths.paths.keys().map(String::as_str).collect();

        let mut missing: Vec<&str> = routes
            .iter()
            .map(String::as_str)
            .filter(|route| !documented.contains(route) && !UNDOCUMENTED.contains(route))
            .collect();
        missing.sort_unstable();
        missing.dedup();

        assert!(
            missing.is_empty(),
            "routes missing from the OpenAPI document (add a #[utoipa::path], \
             or if it genuinely should not be documented add it to \
             UNDOCUMENTED with a reason): {missing:#?}"
        );
    }

    /// Every `.route("<path>"` string literal in an axum route-table source
    /// file, in source order. Duplicates are kept; callers only need set
    /// membership.
    fn extract_route_paths(source: &str) -> Vec<String> {
        const NEEDLE: &str = ".route(";
        let mut paths = Vec::new();
        let mut rest = source;
        while let Some(idx) = rest.find(NEEDLE) {
            rest = &rest[idx + NEEDLE.len()..];
            if let Some(path) = next_string_literal(rest) {
                paths.push(path);
            }
        }
        paths
    }

    /// The contents of the next `"..."` literal in `s`, if any.
    fn next_string_literal(s: &str) -> Option<String> {
        let start = s.find('"')?;
        let after = &s[start + 1..];
        let end = after.find('"')?;
        Some(after[..end].to_string())
    }

    /// Converts axum's path-parameter syntax (`:id`) to OpenAPI's (`{id}`).
    ///
    /// Applied wherever `:name` appears rather than only on whole segments,
    /// because one route embeds its parameter mid-segment:
    /// `/bot:token/sendMessage`.
    fn axum_path_to_openapi(path: &str) -> String {
        let mut out = String::with_capacity(path.len());
        let mut chars = path.chars().peekable();
        while let Some(c) = chars.next() {
            if c != ':' {
                out.push(c);
                continue;
            }
            out.push('{');
            while let Some(&next) = chars.peek() {
                if next.is_ascii_alphanumeric() || next == '_' {
                    out.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            out.push('}');
        }
        out
    }
}
