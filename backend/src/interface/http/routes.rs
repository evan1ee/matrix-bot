use axum::{
    extract::DefaultBodyLimit,
    routing::{delete, get, patch, post, put},
    Router,
};

use super::{
    account, admin, agent, ai, ai_stream, auth, bookmarks, briefing, calendar, finance,
    finance_analytics, finance_budgets, finance_categories, finance_net_worth, finance_receipts,
    footprints, habits, identities, knowledge, knowledge_links, knowledge_transfer, life,
    life_activity, link, mcp, ocr, openapi, portfolio, portfolio_alerts, portfolio_cash,
    portfolio_history, portfolio_prices, recurring_bills, reminders, rooms, settings, sse, storage,
    todo_steps, todos, voice, webhooks,
};

/// The API route table.
///
/// Extracted so tests and the server can drive the real router without duplication.
pub fn api_routes() -> Router<auth::ApiState> {
    Router::new()
        .merge(routes_core())
        .merge(routes_auth())
        .merge(routes_account())
        .merge(routes_productivity())
        .merge(routes_finance())
        .merge(routes_portfolio())
        .merge(routes_life())
        .merge(routes_ai_and_webhooks())
        .merge(routes_agent())
        .merge(routes_admin())
}

fn routes_core() -> Router<auth::ApiState> {
    Router::new()
        .route("/api-docs/openapi.json", get(openapi::openapi_json))
        .route("/rooms", get(rooms::list_rooms))
        .route("/rooms/sync", post(rooms::sync_rooms))
        .route(
            "/settings",
            get(settings::get_settings).put(settings::update_settings),
        )
        .route("/events", get(sse::sse_handler))
        .route("/storage/*key", get(storage::get_storage))
}

fn routes_auth() -> Router<auth::ApiState> {
    Router::new()
        .route(
            "/auth/recovery/redeem",
            post(auth::recovery::redeem_recovery),
        )
        .route(
            "/auth/passkey/enroll/start",
            post(auth::webauthn::enroll_start),
        )
        .route(
            "/auth/passkey/register/start",
            post(auth::webauthn::register_start),
        )
        .route(
            "/auth/passkey/register/finish",
            post(auth::webauthn::register_finish),
        )
        .route(
            "/auth/passkey/login/start",
            post(auth::webauthn::login_start),
        )
        .route(
            "/auth/passkey/login/finish",
            post(auth::webauthn::login_finish),
        )
        .route("/auth/matrix/login", post(auth::matrix::login_with_matrix))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::get_current_user))
        .route("/auth/media", get(auth::media_proxy))
}

fn routes_account() -> Router<auth::ApiState> {
    Router::new()
        .route("/account/passkeys", get(account::passkeys::list_passkeys))
        .route("/calendar/feed.ics", get(calendar::get_feed))
        .route(
            "/account/calendar-token",
            get(calendar::token_status)
                .post(calendar::mint_token)
                .delete(calendar::revoke_token),
        )
        .route(
            "/account/passkeys/add/start",
            post(account::passkeys::add_passkey_start),
        )
        .route(
            "/account/passkeys/:id",
            put(account::passkeys::rename_passkey).delete(account::passkeys::delete_passkey),
        )
        .route("/account/identities", get(identities::list_identities))
        .route(
            "/account/identities/unlink",
            post(identities::unlink_identity),
        )
        .route("/link/inspect", post(link::inspect))
        .route("/link/claim", post(link::claim))
        .route("/link/signup/start", post(link::signup_start))
        .route("/link/signup/finish", post(link::signup_finish))
        .route("/account/sessions", get(account::sessions::list_sessions))
        .route(
            "/account/sessions/revoke-others",
            post(account::sessions::revoke_other_sessions),
        )
        .route(
            "/account/sessions/:id",
            delete(account::sessions::revoke_session),
        )
        .route(
            "/account/recovery-codes",
            post(account::recovery::regenerate_recovery_codes),
        )
        .route(
            "/account/recovery-status",
            get(account::recovery::recovery_status),
        )
        .route(
            "/account/recovery/cancel",
            post(account::recovery::cancel_recovery),
        )
        .route(
            "/account/security-events",
            get(account::audit::list_security_events),
        )
}

fn routes_productivity() -> Router<auth::ApiState> {
    Router::new()
        .route("/briefing/trigger", post(briefing::trigger_briefing))
        .route("/todos", get(todos::list_todos).post(todos::create_todo))
        // Static `tags` and `order` segments outrank the `:id` / `:step_id` params in
        // matchit; `static_segments_win_over_id_params` pins that down.
        .route("/todos/tags", get(todos::list_todo_tags))
        .route(
            "/todos/:id",
            get(todos::get_todo)
                .patch(todos::update_todo)
                .delete(todos::delete_todo),
        )
        .route("/todos/:id/steps", post(todo_steps::add_step))
        .route("/todos/:id/steps/order", put(todo_steps::reorder_steps))
        .route(
            "/todos/:id/steps/:step_id",
            patch(todo_steps::update_step).delete(todo_steps::remove_step),
        )
        .route(
            "/reminders",
            get(reminders::list_reminders).post(reminders::create_reminder),
        )
        .route(
            "/reminders/history",
            delete(reminders::clear_reminder_history),
        )
        .route(
            "/reminders/:id",
            patch(reminders::update_reminder).delete(reminders::delete_reminder),
        )
        .route(
            "/bookmarks",
            get(bookmarks::list_bookmarks).post(bookmarks::create_bookmark),
        )
        .route("/bookmarks/stats", get(bookmarks::get_stats))
        .route(
            "/bookmarks/:id",
            get(bookmarks::get_bookmark)
                .patch(bookmarks::update_bookmark)
                .delete(bookmarks::delete_bookmark),
        )
        .route(
            "/bookmarks/:id/toggle-favorite",
            post(bookmarks::toggle_favorite),
        )
        .route(
            "/bookmarks/:id/toggle-archive",
            post(bookmarks::toggle_archive),
        )
        .route(
            "/habits",
            get(habits::list_habits).post(habits::create_habit),
        )
        .route("/habits/heatmap", get(habits::get_heatmap))
        .route("/habits/stats", get(habits::get_stats))
        .route(
            "/habits/:id",
            put(habits::update_habit).delete(habits::delete_habit),
        )
        .route("/habits/:id/toggle", post(habits::toggle_habit_log))
        .route(
            "/notes",
            get(knowledge::list_notes).post(knowledge::create_note),
        )
        .route("/notes/tags", get(knowledge::list_tags))
        .route("/notes/graph", get(knowledge_links::get_knowledge_graph))
        .route("/notes/:id/links", get(knowledge_links::get_note_links))
        .route("/notes/export", get(knowledge_transfer::export_notes))
        .route(
            "/notes/import",
            post(knowledge_transfer::import_notes).layer(DefaultBodyLimit::max(
                knowledge_transfer::IMPORT_BODY_LIMIT_BYTES,
            )),
        )
        .route(
            "/notes/:id",
            get(knowledge::get_note)
                .put(knowledge::update_note)
                .delete(knowledge::delete_note),
        )
        .route("/knowledge/search", post(knowledge::search_knowledge))
}

fn routes_finance() -> Router<auth::ApiState> {
    Router::new()
        .route(
            "/finance/transactions",
            get(finance::list_transactions).post(finance::create_transaction),
        )
        .route(
            "/finance/transactions/:id",
            put(finance::update_transaction).delete(finance::delete_transaction),
        )
        .route(
            "/finance/transactions/:id/receipt-url",
            get(finance_receipts::receipt_url),
        )
        .route(
            "/finance/transactions/:id/receipt",
            delete(finance_receipts::detach_receipt),
        )
        .route(
            "/finance/receipts/upload",
            post(finance_receipts::upload_receipt).layer(DefaultBodyLimit::max(
                finance_receipts::UPLOAD_BODY_LIMIT_BYTES,
            )),
        )
        .route("/finance/summary", get(finance::summary))
        .route("/finance/net-worth", get(finance_net_worth::get_net_worth))
        .route("/finance/analytics", get(finance_analytics::get_analytics))
        .route(
            "/finance/share",
            get(finance_analytics::get_share)
                .post(finance_analytics::upsert_share)
                .delete(finance_analytics::delete_share),
        )
        .route(
            "/public/finance/:token",
            get(finance_analytics::get_public_report),
        )
        .route(
            "/finance/categories",
            get(finance_categories::list_categories).post(finance_categories::create_category),
        )
        .route(
            "/finance/budgets",
            get(finance_budgets::list_budgets).post(finance_budgets::create_budget),
        )
        .route(
            "/finance/budgets/:id",
            put(finance_budgets::update_budget).delete(finance_budgets::delete_budget),
        )
        .route(
            "/finance/categories/:id",
            delete(finance_categories::delete_category),
        )
        .route(
            "/recurring-bills",
            get(recurring_bills::list_bills).post(recurring_bills::create_bill),
        )
        .route(
            "/recurring-bills/:id",
            get(recurring_bills::get_bill)
                .put(recurring_bills::update_bill)
                .delete(recurring_bills::delete_bill),
        )
        .route("/recurring-bills/:id/pay", post(recurring_bills::pay_bill))
}

fn routes_portfolio() -> Router<auth::ApiState> {
    Router::new()
        .route(
            "/portfolio/holdings",
            get(portfolio::list_holdings).post(portfolio::buy_holding),
        )
        .route("/portfolio/holdings/:id", delete(portfolio::delete_holding))
        .route(
            "/portfolio/holdings/:id/sell",
            post(portfolio::sell_holding),
        )
        .route("/portfolio/holdings/:id/price", put(portfolio::set_price))
        .route(
            "/portfolio/holdings/:id/symbol",
            put(portfolio_prices::update_holding_symbol),
        )
        .route("/portfolio/holdings/:id/note", put(portfolio::set_note))
        .route(
            "/portfolio/refresh-prices",
            post(portfolio_prices::refresh_all_prices),
        )
        .route(
            "/portfolio/holdings/:id/refresh-price",
            post(portfolio_prices::refresh_single_holding_price),
        )
        .route(
            "/portfolio/holdings/record",
            post(portfolio_prices::record_holding_position),
        )
        .route(
            "/portfolio/search",
            get(portfolio_prices::search_ticker_symbols),
        )
        .route("/portfolio/quote", get(portfolio_prices::get_symbol_quote))
        .route(
            "/portfolio/activity",
            get(portfolio_prices::get_portfolio_activity),
        )
        .route(
            "/portfolio/activity/verify",
            get(portfolio_prices::verify_portfolio_activity_chain),
        )
        .route("/portfolio/summary", get(portfolio::summary))
        .route("/portfolio/cash", get(portfolio_cash::list_cash))
        .route(
            "/portfolio/cash/deposit",
            post(portfolio_cash::deposit_cash),
        )
        .route(
            "/portfolio/cash/withdraw",
            post(portfolio_cash::withdraw_cash),
        )
        .route(
            "/portfolio/alerts",
            get(portfolio_alerts::list_alerts).post(portfolio_alerts::create_alert),
        )
        .route(
            "/portfolio/alerts/:id",
            delete(portfolio_alerts::delete_alert),
        )
        .route(
            "/portfolio/history",
            get(portfolio_history::get_portfolio_history),
        )
}

fn routes_life() -> Router<auth::ApiState> {
    Router::new()
        .route(
            "/footprints",
            get(footprints::list_footprints).post(footprints::create_footprint),
        )
        .route("/footprints/stats", get(footprints::get_summary_stats))
        .route("/footprints/geocode", get(footprints::geocode_proxy))
        .route(
            "/footprints/:id",
            get(footprints::get_footprint)
                .put(footprints::update_footprint)
                .delete(footprints::delete_footprint),
        )
        .route(
            "/life/profile",
            get(life::get_profile_and_stats)
                .put(life::upsert_profile)
                .post(life::upsert_profile),
        )
        .route("/life/year/:year", get(life::get_year_view))
        .route("/life/horizon", get(life::get_horizon))
        .route("/life/days", post(life::upsert_day))
        .route(
            "/life/days/:date",
            get(life::get_day).delete(life::delete_day),
        )
        .route("/life/days/:date/time", put(life::update_time_allocation))
        .route(
            "/life/days/:date/activity",
            get(life_activity::get_day_activity),
        )
}

fn routes_ai_and_webhooks() -> Router<auth::ApiState> {
    Router::new()
        .route("/webhooks/:token", post(webhooks::inbound_webhook))
        .route(
            "/bot:token/sendMessage",
            get(webhooks::telegram_send_message_get).post(webhooks::telegram_send_message_post),
        )
        .route(
            "/account/webhooks",
            get(webhooks::list_webhooks).post(webhooks::create_webhook),
        )
        .route(
            "/account/webhooks/:id",
            patch(webhooks::update_webhook).delete(webhooks::delete_webhook),
        )
        .route(
            "/account/webhooks/:id/rotate",
            post(webhooks::rotate_webhook_token),
        )
        .route("/account/webhooks/:id/test", post(webhooks::test_webhook))
        .route(
            "/account/webhooks/:id/logs",
            get(webhooks::get_webhook_logs),
        )
        .route("/ai/chat", post(ai::chat))
        .route("/ai/chat/stream", post(ai_stream::chat_stream))
        .route("/ai/verify", post(ai::verify_ai_config))
        .route(
            "/ai/conversations",
            get(ai::list_conversations).post(ai::create_conversation),
        )
        .route("/ai/conversations/:id", delete(ai::delete_conversation))
        .route(
            "/ai/conversations/:id/title",
            patch(ai::rename_conversation),
        )
        .route(
            "/ai/conversations/:id/truncate",
            post(ai::truncate_conversation),
        )
        .route(
            "/ai/conversations/:id/messages",
            get(ai::get_conversation_messages),
        )
        .route(
            "/account/ai-config",
            get(ai::get_ai_config).put(ai::update_ai_config),
        )
        .route("/ai/retrospective", get(ai::get_retrospective))
        .route("/ocr/scan", post(ocr::scan_image))
        .route("/voice/transcribe", post(voice::transcribe_voice))
        .nest("/mcp", mcp::mcp_routes())
}

fn routes_agent() -> Router<auth::ApiState> {
    Router::new()
        .route("/agent/runs", get(agent::list_runs).post(agent::start_run))
        .route("/agent/runs/:id", get(agent::get_run))
        .route("/agent/runs/:id/cancel", post(agent::cancel_run))
        .route("/agent/approvals", get(agent::list_pending_approvals))
        .route("/agent/approvals/:id/approve", post(agent::approve_action))
        .route("/agent/approvals/:id/reject", post(agent::reject_action))
        .route("/agent/tasks", get(agent::list_tasks))
        .route(
            "/agent/tasks/:id",
            put(agent::schedule_task).delete(agent::delete_task),
        )
        .route("/agent/tasks/:id/state", patch(agent::set_task_state))
        .route("/agent/tasks/:id/trigger", post(agent::trigger_task))
}

fn routes_admin() -> Router<auth::ApiState> {
    Router::new()
        .route("/admin/config-status", get(admin::status::config_status))
        .route("/admin/settings", get(admin::settings::list_settings))
        .route(
            "/admin/settings/:key",
            get(admin::settings::get_setting)
                .put(admin::settings::update_setting)
                .delete(admin::settings::reset_setting),
        )
        .route(
            "/admin/secrets",
            get(admin::secrets::list_secrets).post(admin::secrets::create_secret),
        )
        .route(
            "/admin/secrets/:id",
            put(admin::secrets::replace_secret).delete(admin::secrets::delete_secret),
        )
        .route("/admin/secrets/:id/test", post(admin::secrets::test_secret))
        .route("/admin/audit-logs", get(admin::audit::list_audit_logs))
}
