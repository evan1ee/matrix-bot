use axum::{response::IntoResponse, Json};
use utoipa::openapi::security::SecurityScheme;
use utoipa::{Modify, OpenApi};

use crate::interface::http::error::ApiErrorResponse;
use crate::interface::http::finance::{CategoryTotal, CurrencySummary, Direction, Transaction};
use crate::interface::http::finance_analytics::{
    ExpenseCategorySlice, FinanceAnalyticsReport, FinanceShare, MonthlyCashflowBucket,
    PublicFinanceReport, PublicTransaction,
};
use crate::interface::http::finance_categories::Category;
use crate::interface::http::finance_net_worth::{CurrencyNetWorthBreakdown, NetWorthResponse};
use crate::interface::http::footprints::{
    CreateFootprintRequest, Footprint, FootprintCurrencySpend, FootprintsSummary,
    UpdateFootprintRequest,
};
use crate::interface::http::habits::{
    CreateHabitRequest, DayStatus, Habit, HabitStatsSummary, HabitWithStatus, HeatmapDay,
    ToggleHabitRequest, UpdateHabitRequest,
};
use crate::interface::http::life::{
    LifeDayLog, LifeHorizonDay, LifeProfile, LifeProfileStats, LifeYearView,
    UpdateTimeAllocationInput, UpsertLifeDayLogInput, UpsertLifeProfileInput,
};
use crate::interface::http::life_activity::{ActivityEvent, ActivityStats, DayActivityResponse};
use crate::interface::http::portfolio::{CashBalance, Holding, PortfolioCurrencySummary};
use crate::interface::http::recurring_bills::{
    CreateBillPayload, RecurringBill, UpdateBillPayload,
};
use crate::interface::http::reminders::{RecurrencePattern, Reminder, ReminderStatus};
use crate::interface::http::todos::{TodoItem, TodoPriority};
use crate::interface::http::webhooks::{TelegramMessageResult, TelegramQuery, TelegramResponse};
use crate::interface::http::{
    account, admin, agent, ai, ai_stream, auth, bookmarks, briefing, finance, finance_analytics,
    finance_budgets, finance_categories, finance_net_worth, footprints, habits, health, identities,
    knowledge, knowledge_transfer, life, life_activity, link, mcp, ocr, portfolio,
    portfolio_alerts, portfolio_cash, portfolio_history, portfolio_prices, recurring_bills,
    reminders, rooms, settings, sse, storage, todo_steps, todos, voice, webhooks,
};

#[derive(OpenApi)]
#[openapi(
    paths(
        health::live,
        health::ready,
        sse::sse_handler,
        storage::get_storage,
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
        identities::list_identities,
        identities::unlink_identity,
        link::inspect,
        link::claim,
        link::signup_start,
        link::signup_finish,
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
        todos::get_todo,
        todos::list_todo_tags,
        todo_steps::add_step,
        todo_steps::update_step,
        todo_steps::remove_step,
        todo_steps::reorder_steps,
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
        finance_categories::list_categories,
        finance_categories::create_category,
        finance_categories::delete_category,
        finance_budgets::list_budgets,
        finance_budgets::create_budget,
        finance_budgets::update_budget,
        finance_budgets::delete_budget,
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
        ai_stream::chat_stream,
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
        knowledge::list_notes,
        knowledge::list_tags,
        knowledge::get_note,
        knowledge::create_note,
        knowledge::update_note,
        knowledge::delete_note,
        knowledge::search_knowledge,
        knowledge_transfer::export_notes,
        knowledge_transfer::import_notes,
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
        webhooks::telegram_compat::telegram_send_message_get,
        webhooks::telegram_compat::telegram_send_message_post,
        agent::start_run,
        agent::list_runs,
        agent::get_run,
        agent::cancel_run,
        agent::list_pending_approvals,
        agent::approve_action,
        agent::reject_action,
        agent::list_tasks,
        agent::schedule_task,
        admin::status::config_status,
        admin::settings::list_settings,
        admin::settings::get_setting,
        admin::settings::update_setting,
        admin::settings::reset_setting,
        admin::secrets::list_secrets,
        admin::secrets::create_secret,
        admin::secrets::replace_secret,
        admin::secrets::delete_secret,
        admin::secrets::test_secret,
        admin::audit::list_audit_logs,
    ),
    components(
        schemas(
            agent::AgentRunResponse,
            agent::AgentStepResponse,
            agent::AgentRunDetailResponse,
            agent::ApprovalRequestResponse,
            agent::ScheduledTaskResponse,
            agent::StartAgentRunRequest,
            agent::RejectActionRequest,
            agent::ScheduleTaskRequest,
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
            identities::IdentityDto,
            identities::IdentityListResponse,
            identities::UnlinkIdentityRequest,
            identities::UnlinkIdentityResponse,
            link::LinkTokenRequest,
            link::LinkSignupFinishRequest,
            link::LinkPreviewState,
            link::LinkPreviewDto,
            link::LinkClaimOutcome,
            link::LinkClaimResponse,
            account::RecoveryCodesResponse,
            account::RenamePasskeyRequest,
            account::SecurityEvent,
            auth::UserProfileResponse,
            auth::MediaProxyQuery,
            rooms::JoinedRoomInfo,
            settings::UserSettings,
            settings::UpdateSettingsRequest,
            briefing::BriefingPayload,
            briefing::BriefingSummary,
            briefing::TickerPerformance,
            TodoItem,
            TodoPriority,
            todos::TodoQuery,
            todos::CreateTodoRequest,
            todos::UpdateTodoRequest,
            todos::TodoListResponse,
            todos::TodoDetail,
            todos::TodoStepDto,
            todos::TodoTagsResponse,
            todos::TodoTagCount,
            todo_steps::CreateStepRequest,
            todo_steps::UpdateStepRequest,
            todo_steps::ReorderStepsRequest,
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
            finance_budgets::BudgetDto,
            finance_budgets::BudgetStatusDto,
            finance_budgets::BudgetScopeDto,
            finance_budgets::BudgetStateDto,
            finance_budgets::PaceDto,
            finance_budgets::BudgetQuery,
            finance_budgets::CreateBudgetRequest,
            finance_budgets::UpdateBudgetRequest,
            NetWorthResponse,
            CurrencyNetWorthBreakdown,
            MonthlyCashflowBucket,
            ExpenseCategorySlice,
            FinanceAnalyticsReport,
            FinanceShare,
            PublicTransaction,
            PublicFinanceReport,
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
            portfolio_prices::RefreshSummary,
            portfolio_prices::TickerSearchResult,
            portfolio_prices::UpdateSymbolRequest,
            portfolio_prices::SymbolQuoteResponse,
            portfolio_alerts::CreateAlertRequest,
            portfolio_alerts::PriceAlert,
            portfolio_alerts::AlertCondition,
            portfolio_history::PortfolioHistoryResponse,
            portfolio_history::PortfolioHistoryPoint,
            portfolio_prices::PortfolioAuditEntry,
            portfolio_prices::AuditVerificationResult,
            RecurringBill,
            CreateBillPayload,
            UpdateBillPayload,
            crate::interface::http::webhooks::Webhook,
            crate::interface::http::webhooks::WebhookOutboxItem,
            crate::interface::http::webhooks::WebhookSeverity,
            crate::interface::http::webhooks::CreatedWebhookResponse,
            crate::interface::http::webhooks::CreateWebhookInput,
            crate::interface::http::webhooks::UpdateWebhookInput,
            crate::interface::http::webhooks::InboundPayload,
            crate::interface::http::webhooks::InboundAcceptedResponse,
            crate::interface::http::webhooks::WebhookLogsResponse,
            crate::llm::models::AiUserConfig,
            crate::llm::models::UpdateAiConfigInput,
            crate::llm::models::VerifyAiKeyInput,
            crate::llm::models::VerifyAiKeyResponse,
            crate::llm::models::AiConversation,
            crate::llm::models::AiMessage,
            crate::llm::models::AiToolOutcome,
            crate::llm::models::AiChatRequest,
            crate::llm::models::AiChatResponse,
            crate::interface::http::ai_stream::AiStreamEvent,
            crate::llm::retrospective::WeeklyRetrospective,
            crate::llm::retrospective::CurrencySpendSummary,
            crate::llm::retrospective::UpcomingBillsSummary,
            bookmarks::Bookmark,
            bookmarks::CreateBookmarkInput,
            bookmarks::UpdateBookmarkInput,
            bookmarks::BookmarkStats,
            bookmarks::DomainCount,
            knowledge::Note,
            knowledge::NoteSummary,
            knowledge::CreateNoteInput,
            knowledge::UpdateNoteInput,
            knowledge::KnowledgeSearchRequest,
            knowledge::SearchResultItem,
            knowledge_transfer::ImportReportDto,
            knowledge_transfer::SkippedFileDto,
            crate::interface::mcp::models::CreateMcpTokenRequest,
            crate::interface::mcp::models::CreateMcpTokenResponse,
            crate::interface::mcp::models::McpTokenSummary,
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
            CreateHabitRequest,
            UpdateHabitRequest,
            ToggleHabitRequest,
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
            TelegramMessageResult,
            admin::status::ConfigurationStatusDto,
            admin::settings::SettingItemDto,
            admin::settings::UpdateSettingRequest,
            admin::settings::ResetSettingResponse,
            admin::secrets::SecretMetadataDto,
            admin::secrets::CreateSecretRequest,
            admin::secrets::ReplaceSecretRequest,
            admin::secrets::DeleteSecretResponse,
            admin::secrets::SecretTestResultDto,
            admin::audit::ConfigAuditRowDto,
            admin::audit::AuditLogPageDto,
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
                    crate::domain::accounts::COOKIE_NAME,
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
            if c != ':' && c != '*' {
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
