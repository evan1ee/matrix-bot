use crate::application::AppServices;
use crate::domain::accounts::AccountId;
use crate::domain::finance::money::{
    exponent_for, looks_like_currency, normalise_currency, parse_amount,
};
use crate::domain::portfolio::alerts::AlertCondition;
use crate::domain::portfolio::models::normalise_symbol;
use crate::utils::html::html_escape;

pub async fn handle_alert_command(services: &AppServices, owner: &AccountId, args: &str) -> String {
    let trimmed = args.trim();
    let lower = trimmed.to_lowercase();

    if trimmed.is_empty() || lower.starts_with("list") || lower == "s" {
        return handle_list_alerts(services, owner).await;
    }

    if lower.starts_with("cancel") || lower.starts_with("delete") || lower.starts_with("rm") {
        let rest = trimmed
            .split_whitespace()
            .skip(1)
            .collect::<Vec<_>>()
            .join(" ");
        return handle_cancel_alert(services, owner, &rest).await;
    }

    handle_create_alert(services, owner, trimmed).await
}

async fn handle_list_alerts(services: &AppServices, owner: &AccountId) -> String {
    let alerts = match services.portfolio.list_alerts(owner).await {
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

async fn handle_cancel_alert(services: &AppServices, owner: &AccountId, raw_id: &str) -> String {
    let Ok(id) = raw_id.trim().parse::<i64>() else {
        return "<p>Please specify a valid numeric alert ID. Example: <code>!portfolio alert cancel 3</code></p>".to_string();
    };

    match services.portfolio.delete_alert(owner, id).await {
        Ok(true) => format!("<p>Price alert #{} cancelled.</p>", id),
        Ok(false) => format!("<p>Alert #{} not found or does not belong to you.</p>", id),
        Err(err) => format!(
            "<p>Failed to cancel alert #{}: {}</p>",
            id,
            html_escape(&err.to_string())
        ),
    }
}

async fn handle_create_alert(services: &AppServices, owner: &AccountId, args: &str) -> String {
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

    let condition = match AlertCondition::parse(tokens[1]) {
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
        let settings = services.settings.get_or_default(owner).await;
        let exp = i64::from(exponent_for(&settings.currency));
        (settings.currency, exp)
    };

    let target_price_minor = match parse_amount(raw_price, exponent as u32) {
        Ok(p) if p > 0 => p,
        _ => return "<p>Invalid price. Enter a positive decimal number, e.g. <code>150.00</code>.</p>".to_string(),
    };

    match services
        .portfolio
        .create_alert(
            owner,
            &symbol,
            target_price_minor,
            &currency,
            exponent,
            condition,
        )
        .await
    {
        Ok(alert) => {
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
