use axum::{
    routing::{delete, get, patch, post, put},
    Router,
};

use super::{
    account, ai, auth, bookmarks, finance, footprints, habits, health, life, life_activity,
    openapi, portfolio, portfolio_alerts, portfolio_cash, portfolio_history, portfolio_prices,
    recurring_bills, reminders, rooms, settings, sse, todos, webhooks,
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
}

fn routes_core() -> Router<auth::ApiState> {
    Router::new()
        .route("/health", get(health::health_check))
        .route("/api-docs/openapi.json", get(openapi::openapi_json))
        .route("/rooms", get(rooms::list_rooms))
        .route("/rooms/sync", post(rooms::sync_rooms))
        .route(
            "/settings",
            get(settings::get_settings).put(settings::update_settings),
        )
        .route("/events", get(sse::sse_handler))
}

fn routes_auth() -> Router<auth::ApiState> {
    Router::new()
        .route(
            "/auth/recovery/redeem",
            post(auth::recovery::redeem_recovery),
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
        .route(
            "/account/passkeys/:id",
            put(account::passkeys::rename_passkey).delete(account::passkeys::delete_passkey),
        )
        .route("/account/matrix", get(account::links::list_links))
        .route("/account/matrix/link", post(account::links::start_link))
        .route(
            "/account/matrix/pending-merge",
            get(account::links::pending_merge),
        )
        .route(
            "/account/matrix/confirm-merge",
            post(account::links::confirm_merge),
        )
        .route("/account/matrix/unlink", post(account::links::unlink))
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
        .route("/todos", get(todos::list_todos).post(todos::create_todo))
        .route(
            "/todos/:id",
            patch(todos::update_todo).delete(todos::delete_todo),
        )
        .route(
            "/reminders",
            get(reminders::list_reminders).post(reminders::create_reminder),
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
        .route("/finance/summary", get(finance::summary))
        .route(
            "/finance/categories",
            get(finance::list_categories).post(finance::create_category),
        )
        .route("/finance/categories/:id", delete(finance::delete_category))
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
        .route("/ai/verify", post(ai::verify_ai_config))
        .route("/ai/conversations", get(ai::list_conversations))
        .route("/ai/conversations/:id", delete(ai::delete_conversation))
        .route(
            "/ai/conversations/:id/messages",
            get(ai::get_conversation_messages),
        )
        .route(
            "/account/ai-config",
            get(ai::get_ai_config).put(ai::update_ai_config),
        )
        .route("/ai/retrospective", get(ai::get_retrospective))
}
