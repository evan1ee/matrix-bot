use crate::db::DbPool;
use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::finance::money::{
    exponent_for, looks_like_currency, normalise_currency, parse_amount,
};
use crate::modules::portfolio::alerts::{AlertCondition, AlertsRepository};
use crate::modules::portfolio::models::normalise_symbol;
use crate::utils::html_escape;

pub async fn handle_alert_command(
    pool: &DbPool,
    settings: &crate::application::settings::SettingsService,
    owner_id: &str,
    args: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    let trimmed = args.trim();
    let lower = trimmed.to_lowercase();

    if trimmed.is_empty() || lower.starts_with("list") || lower == "s" {
        return handle_list_alerts(pool, owner_id).await;
    }

    if lower.starts_with("cancel") || lower.starts_with("delete") || lower.starts_with("rm") {
        let rest = trimmed
            .split_whitespace()
            .skip(1)
            .collect::<Vec<_>>()
            .join(" ");
        return handle_cancel_alert(pool, owner_id, &rest, broadcaster).await;
    }

    handle_create_alert(pool, settings, owner_id, trimmed, broadcaster).await
}

async fn handle_list_alerts(pool: &DbPool, owner_id: &str) -> String {
    let alerts = match AlertsRepository::list_for_owner(pool, owner_id).await {
        Ok(list) => list,
        Err(err) => {
            return format!(
                "Could not load price alerts: {}",
                html_escape(&err.to_string())
            )
        }
    };

    if alerts.is_empty() {
        return "<p>No price alerts set. Use <code>!portfolio alert &lt;symbol&gt; &gt; &lt;price&gt;</code> to set one.</p>".to_string();
    }

    let mut out = String::from("<p><strong>Your Price Alerts:</strong></p><ul>");
    for a in alerts {
        let status = if a.is_triggered {
            "<em>(Triggered)</em>"
        } else {
            "<strong>Active</strong>"
        };
        let target_fmt = (a.target_price_minor as f64) / 10f64.powi(a.currency_exponent as i32);
        out.push_str(&format!(
            "<li>#{} <strong>{}</strong> {} {:.2} {} — {}</li>",
            a.id,
            html_escape(&a.symbol),
            a.parsed_condition().symbol_str(),
            target_fmt,
            html_escape(&a.currency),
            status
        ));
    }
    out.push_str("</ul>");
    out
}

async fn handle_cancel_alert(
    pool: &DbPool,
    owner_id: &str,
    raw_id: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    let Ok(id) = raw_id.trim().parse::<i64>() else {
        return "<p>Please specify a valid numeric alert ID. Example: <code>!portfolio alert cancel 3</code></p>".to_string();
    };

    match AlertsRepository::delete(pool, id, owner_id).await {
        Ok(true) => {
            if let Some(b) = broadcaster {
                b.send(SystemEvent::PortfolioAlertDeleted {
                    id,
                    owner_id: owner_id.to_string(),
                });
            }
            format!("<p>Price alert #{} cancelled.</p>", id)
        }
        Ok(false) => format!("<p>Alert #{} not found or does not belong to you.</p>", id),
        Err(err) => format!(
            "<p>Failed to cancel alert #{}: {}</p>",
            id,
            html_escape(&err.to_string())
        ),
    }
}

async fn handle_create_alert(
    pool: &DbPool,
    settings: &crate::application::settings::SettingsService,
    owner_id: &str,
    args: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    // Grammar: <symbol> <condition> <price> [currency]
    // Condition can be >, <, above, below, >=, <=
    let tokens: Vec<&str> = args.split_whitespace().collect();
    if tokens.len() < 3 {
        return "<p>Usage: <code>!portfolio alert &lt;symbol&gt; &gt; &lt;price&gt; [currency]</code><br/>\
                Example: <code>!portfolio alert BTC-USD &gt; 70000</code> or <code>!portfolio alert NVDA &lt; 120 USD</code></p>".to_string();
    }

    let symbol = match normalise_symbol(tokens[0]) {
        Ok(s) => s,
        Err(err) => return format!("<p>Invalid symbol: {}</p>", html_escape(&err.to_string())),
    };

    let condition = match AlertCondition::from_str(tokens[1]) {
        Some(c) => c,
        None => return format!("<p>Invalid condition '{}'. Use <code>&gt;</code> (above) or <code>&lt;</code> (below).</p>", html_escape(tokens[1])),
    };

    let raw_price = tokens[2];
    let (currency, exponent) = if tokens.len() >= 4 && looks_like_currency(tokens[3]) {
        let cur = match normalise_currency(tokens[3]) {
            Ok(c) => c,
            Err(e) => return format!("<p>{}</p>", html_escape(&e.to_string())),
        };
        let exp = i64::from(exponent_for(&cur));
        (cur, exp)
    } else {
        let settings = settings.get_or_default(owner_id).await;
        let exp = i64::from(exponent_for(&settings.currency));
        (settings.currency, exp)
    };

    let target_price_minor = match parse_amount(raw_price, exponent as u32) {
        Ok(p) if p > 0 => p,
        _ => return "<p>Invalid price. Enter a positive decimal number, e.g. <code>150.00</code>.</p>".to_string(),
    };

    match AlertsRepository::create(
        pool,
        owner_id,
        &symbol,
        target_price_minor,
        &currency,
        exponent,
        condition,
    )
    .await
    {
        Ok(alert) => {
            if let Some(b) = broadcaster {
                b.send(SystemEvent::PortfolioAlertCreated(alert.clone()));
            }
            let target_fmt = (target_price_minor as f64) / 10f64.powi(exponent as i32);
            format!(
                "<p>🔔 Alert #{} created: will notify you when <strong>{}</strong> goes <strong>{}</strong> {:.2} {}.</p>",
                alert.id,
                html_escape(&symbol),
                condition.as_str(),
                target_fmt,
                html_escape(&currency)
            )
        }
        Err(err) => format!(
            "<p>Failed to create alert: {}</p>",
            html_escape(&err.to_string())
        ),
    }
}
