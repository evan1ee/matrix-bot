//! Rendering a portfolio: the read-only commands and the formatting they share.
//!
//! Split from `handler.rs` along the seam between showing a portfolio and
//! changing one. Nothing here writes, so a change to how a holding reads cannot
//! affect the money arithmetic in `handler.rs` or `repository.rs`.
//!
//! Market value and unrealised P&L stay `Option` rather than defaulting to
//! zero: there is no price feed in this project, so a holding whose price has
//! never been set has genuinely unknown value, and rendering that as 0 would be
//! a lie the user cannot distinguish from a real loss.

use sqlx::SqlitePool;
use tracing::error;

use crate::modules::finance::money::format_amount;
use crate::modules::portfolio::models::{Holding, QUANTITY_EXPONENT};
use crate::modules::portfolio::repository::PortfolioRepository;
use crate::utils::html_escape;

pub fn usage() -> String {
    "⚠️ Unknown command. Try <code>!portfolio buy &lt;symbol&gt; &lt;qty&gt; @ &lt;price&gt; [CUR] [note]</code>, \
     <code>!portfolio record &lt;symbol&gt; &lt;qty&gt; @ &lt;price&gt; [CUR] [note]</code>, \
     <code>!portfolio sell &lt;symbol&gt; &lt;qty&gt; @ &lt;price&gt;</code>, \
     <code>!portfolio price &lt;symbol&gt; &lt;price&gt;</code>, <code>!portfolio refresh</code>, \
     <code>!portfolio note &lt;symbol&gt; [text]</code>, \
     <code>!portfolio cash</code>, <code>!portfolio cash deposit &lt;amount&gt; [CUR]</code>, \
     <code>!portfolio cash withdraw &lt;amount&gt; [CUR]</code>, \
     <code>!portfolio list</code>, <code>!portfolio summary</code>, or <code>!portfolio remove &lt;symbol&gt;</code>."
        .to_string()
}

/// Renders a quantity, trimming trailing zeros beyond what was actually typed —
/// `format_amount` alone would print every one of the 8 stored decimal places,
/// which reads as noise for a share count like `10.00000000`.
pub fn format_quantity(minor: i64) -> String {
    let full = format_amount(minor, QUANTITY_EXPONENT);
    if let Some((whole, frac)) = full.split_once('.') {
        let trimmed = frac.trim_end_matches('0');
        if trimmed.is_empty() {
            whole.to_string()
        } else {
            format!("{}.{}", whole, trimmed)
        }
    } else {
        full
    }
}

pub fn signed_amount(minor: i64, exponent: u32) -> String {
    let sign = if minor < 0 { "-" } else { "+" };
    format!("{}{}", sign, format_amount(minor.abs(), exponent))
}

pub async fn handle_list(pool: &SqlitePool, owner_id: &str) -> String {
    match PortfolioRepository::list(pool, owner_id).await {
        Ok(holdings) if holdings.is_empty() => {
            "📈 No holdings yet. Add one with <code>!portfolio buy AAPL 10 @ 150.00</code>.".to_string()
        }
        Ok(holdings) => {
            let mut html = "📈 <b>Your holdings:</b><br/>".to_string();
            for h in &holdings {
                html.push_str(&format!("• {}<br/>", format_holding_line(h)));
            }
            html
        }
        Err(err) => {
            error!("Failed to list holdings: {:?}", err);
            "❌ Database error loading your holdings.".to_string()
        }
    }
}

pub fn format_holding_line(h: &Holding) -> String {
    let exp = h.currency_exponent as u32;
    let qty = html_escape(&format_quantity(h.quantity_minor));
    let avg = html_escape(&format_amount(h.avg_cost_minor, exp));
    let base = format!(
        "<b>{}</b> — {} @ avg {} {}",
        html_escape(&h.symbol),
        qty,
        avg,
        html_escape(&h.currency)
    );

    let priced = match (h.market_value_minor(), h.unrealized_pl_minor()) {
        (Some(value), Some(pl)) => format!(
            " — now {} {} ({} {})",
            html_escape(&format_amount(value, exp)),
            html_escape(&h.currency),
            signed_amount(pl, exp),
            html_escape(&h.currency)
        ),
        _ => " — no current price set".to_string(),
    };

    let note = h
        .note
        .as_deref()
        .map(|n| format!(" <i>— {}</i>", html_escape(n)))
        .unwrap_or_default();

    format!("{}{}{}", base, priced, note)
}

pub async fn handle_summary(pool: &SqlitePool, owner_id: &str) -> String {
    match PortfolioRepository::summary(pool, owner_id).await {
        Ok(summaries) if summaries.is_empty() => "📊 No holdings recorded yet.".to_string(),
        Ok(summaries) => {
            let mut html = "📊 <b>Portfolio summary:</b><br/>".to_string();
            for s in summaries {
                let exp = s.currency_exponent as u32;
                html.push_str(&format!(
                    "<br/><b>{}</b> — cost {} / value {} / P&amp;L {}",
                    html_escape(&s.currency),
                    html_escape(&format_amount(s.cost_basis_minor, exp)),
                    html_escape(&format_amount(s.market_value_minor, exp)),
                    signed_amount(s.unrealized_pl_minor, exp),
                ));
                if s.unpriced_holdings > 0 {
                    html.push_str(&format!(
                        " <i>({} holding{} with no price set)</i>",
                        s.unpriced_holdings,
                        if s.unpriced_holdings == 1 { "" } else { "s" }
                    ));
                }
                html.push_str("<br/>");
            }
            html
        }
        Err(err) => {
            error!("Failed to summarise holdings: {:?}", err);
            "❌ Database error building the summary.".to_string()
        }
    }
}
