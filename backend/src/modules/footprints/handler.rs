use sqlx::SqlitePool;
use tracing::error;

use super::repository::FootprintRepository;
use crate::events::{EventBroadcaster, SystemEvent};
use crate::utils::html::html_escape;

pub async fn handle_footprint_command(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    _room_id: &str,
    owner_id: &str,
    _sender_name: &str,
    command: &str,
    raw_args: &str,
    _full_body: &str,
    _is_direct: bool,
) -> Option<String> {
    let cmd = command.to_lowercase();
    if cmd != "footprint" && cmd != "footprints" && cmd != "places" {
        return None;
    }

    let trimmed = raw_args.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("list") {
        Some(handle_list(pool, owner_id).await)
    } else if let Some(rest) = trimmed
        .strip_prefix("delete")
        .or_else(|| trimmed.strip_prefix("rm"))
    {
        Some(handle_delete(pool, broadcaster, owner_id, rest.trim()).await)
    } else if let Some(rest) = trimmed.strip_prefix("add") {
        Some(handle_add(pool, broadcaster, owner_id, rest.trim()).await)
    } else {
        Some(format_help())
    }
}

async fn handle_list(pool: &SqlitePool, owner_id: &str) -> String {
    let footprints =
        match FootprintRepository::list_by_owner(pool, owner_id, None, None, None).await {
            Ok(list) => list,
            Err(e) => {
                error!("Failed to list footprints: {}", e);
                return "<p>⚠️ Failed to load your footprints.</p>".to_string();
            }
        };

    let summary = FootprintRepository::summary_stats(pool, owner_id)
        .await
        .unwrap_or_else(|_| super::models::FootprintsSummary {
            total_places: footprints.len() as i64,
            total_countries: 0,
            total_days_traveled: 0,
            spend_by_currency: vec![],
        });

    if footprints.is_empty() {
        return "<p>🌍 <strong>No footprints recorded yet.</strong><br/>\
                Use <code>!footprint add &lt;Place Name&gt; [| YYYY-MM-DD]</code> \
                or drop a star on the interactive web map!</p>"
            .to_string();
    }

    let mut out = format!(
        "<p>🌍 <strong>Your Footprints &amp; Travel Journey</strong><br/>\
         📍 <strong>{}</strong> places visited across 🏳️ <strong>{}</strong> countries • 🗓️ <strong>{}</strong> days traveled</p><ul>",
        summary.total_places, summary.total_countries, summary.total_days_traveled
    );

    for fp in footprints.iter().take(10) {
        let stars = "⭐".repeat(fp.rating.clamp(1, 5) as usize);
        let loc = match (&fp.city, &fp.country) {
            (Some(c), Some(co)) => format!(" ({}, {})", html_escape(c), html_escape(co)),
            (Some(c), None) => format!(" ({})", html_escape(c)),
            (None, Some(co)) => format!(" ({})", html_escape(co)),
            (None, None) => String::new(),
        };
        let dep = fp
            .departure_date
            .as_deref()
            .map(|d| format!(" → {}", d))
            .unwrap_or_default();
        out.push_str(&format!(
            "<li><strong>#{}</strong>: <strong>{}</strong>{} — <em>{}</em> ({} {}{})</li>",
            fp.id,
            html_escape(&fp.title),
            loc,
            stars,
            fp.arrival_date,
            dep,
            if fp.stay_duration_days > 1 {
                format!(", {} days", fp.stay_duration_days)
            } else {
                "".to_string()
            }
        ));
    }

    out.push_str("</ul>");
    if footprints.len() > 10 {
        out.push_str(&format!(
            "<p><em>...and {} more places on your interactive map.</em></p>",
            footprints.len() - 10
        ));
    }

    out
}

async fn handle_add(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    args: &str,
) -> String {
    if args.is_empty() {
        return "<p>Usage: <code>!footprint add &lt;Place Name&gt; [| YYYY-MM-DD] [| Rating 1-5] [| Note]</code></p>".to_string();
    }

    let parts: Vec<&str> = args.split('|').map(|s| s.trim()).collect();
    let title = parts[0];
    if title.is_empty() {
        return "<p>Place name cannot be empty.</p>".to_string();
    }
    if title.len() > 500 {
        return "<p>⚠️ Place name exceeds maximum length of 500 characters.</p>".to_string();
    }

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let arrival_date: &str = parts
        .get(1)
        .filter(|s| !s.is_empty())
        .copied()
        .unwrap_or(&today);
    let rating = parts
        .get(2)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(5)
        .clamp(1, 5);
    let note = parts.get(3).copied();
    if let Some(n) = note {
        if n.len() > 500 {
            return "<p>⚠️ Note exceeds maximum length of 500 characters.</p>".to_string();
        }
    }

    // Default to 0,0 if added via text command without coordinates (user can set coordinates later on map)
    match FootprintRepository::create(
        pool,
        owner_id,
        title,
        0.0,
        0.0,
        None,
        None,
        None,
        arrival_date,
        None,
        0,
        "USD",
        2,
        rating,
        "sightseeing",
        note,
    )
    .await
    {
        Ok(fp) => {
            if let Some(b) = broadcaster {
                b.send(SystemEvent::FootprintCreated(fp.clone()));
            }
            format!(
                "<p>🌟 <strong>Footprint Recorded!</strong><br/>\
                 Added <strong>{}</strong> (Arrival: {}) {}<br/>\
                 <em>Open the web map to drop an exact star pin!</em></p>",
                html_escape(&fp.title),
                fp.arrival_date,
                "⭐".repeat(fp.rating as usize)
            )
        }
        Err(e) => {
            error!("Failed to add footprint: {}", e);
            "<p>⚠️ Could not save footprint. Please check the format.</p>".to_string()
        }
    }
}

async fn handle_delete(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    id_str: &str,
) -> String {
    let id: i64 = match id_str.parse() {
        Ok(v) => v,
        Err(_) => return "<p>Usage: <code>!footprint delete &lt;id&gt;</code></p>".to_string(),
    };

    match FootprintRepository::delete(pool, id, owner_id).await {
        Ok(true) => {
            if let Some(b) = broadcaster {
                b.send(SystemEvent::FootprintDeleted {
                    id,
                    owner_id: owner_id.to_string(),
                });
            }
            format!("<p>🗑️ Footprint #{} deleted.</p>", id)
        }
        Ok(false) => format!("<p>Footprint #{} not found or not owned by you.</p>", id),
        Err(e) => {
            error!("Failed to delete footprint: {}", e);
            "<p>⚠️ Failed to delete footprint.</p>".to_string()
        }
    }
}

fn format_help() -> String {
    "<p><strong>🌍 Footprints Commands:</strong><br/>\
     • <code>!footprint list</code> — View your visited places &amp; journey stats<br/>\
     • <code>!footprint add &lt;name&gt; [| YYYY-MM-DD] [| rating 1-5] [| note]</code> — Record a new visited place<br/>\
     • <code>!footprint delete &lt;id&gt;</code> — Delete a footprint<br/>\
     <em>Tip: You can click anywhere on the web interactive map to drop star markers!</em></p>".to_string()
}
