use chrono::{Timelike, Utc};
use matrix_sdk::ruma::{events::room::message::RoomMessageEventContent, RoomId};
use std::time::Duration as StdDuration;
use tracing::{error, info, warn};

use crate::domain::settings::parse_briefing_time;
use crate::events::SystemEvent;
use crate::modules::ModuleContext;

/// Checks and delivers automated morning briefings for active users.
pub async fn run_briefing_tick(ctx: &ModuleContext) {
    let accounts = match ctx.services.accounts.list_active_ids().await {
        Ok(list) => list,
        Err(err) => {
            error!(
                "Error fetching active accounts for morning briefing: {:?}",
                err
            );
            return;
        }
    };

    let now_utc = Utc::now();

    for owner_id in accounts {
        let settings = ctx.services.settings.get_or_default(&owner_id).await;
        if !settings.briefing_enabled {
            continue;
        }

        let (target_h, target_m) = match parse_briefing_time(&settings.briefing_time) {
            Ok(t) => t,
            Err(_) => (7, 0), // fallback to 07:00
        };

        let tz = settings.tz();
        let now_local = now_utc.with_timezone(&tz);
        let today_local_str = now_local.format("%Y-%m-%d").to_string();

        // 1. Check if already delivered today
        if let Some(last_date) = ctx
            .services
            .settings
            .get_last_briefing_date(&owner_id)
            .await
        {
            if last_date == today_local_str {
                continue;
            }
        }

        // 2. Check if current wall clock is within the delivery window (target minute up to +5 mins)
        let is_target_hour = now_local.hour() == target_h;
        let is_target_window = now_local.minute() >= target_m && now_local.minute() < target_m + 5;

        if !is_target_hour || !is_target_window {
            continue;
        }

        info!(
            "Generating scheduled morning briefing for {} (time: {:02}:{:02}, local: {:02}:{:02})",
            owner_id,
            target_h,
            target_m,
            now_local.hour(),
            now_local.minute()
        );

        let payload = match ctx
            .services
            .briefing
            .generate_briefing(owner_id.as_str(), &settings)
            .await
        {
            Ok(p) => p,
            Err(err) => {
                error!(
                    "Failed generating morning briefing for {}: {:?}",
                    owner_id, err
                );
                continue;
            }
        };

        let mut delivered = false;

        // 3. Deliver to user's direct or default Matrix room
        if let Some(target_room) = ctx.services.rooms.default_room(&owner_id).await {
            if let Ok(room_id) = <&RoomId>::try_from(target_room.as_str()) {
                let content =
                    RoomMessageEventContent::text_html(&payload.plain_text, &payload.html_content);
                if let Some(room) = ctx.client.get_room(room_id) {
                    if let Err(err) = room.send(content.clone()).await {
                        warn!(
                            "Failed sending briefing to room {}: {:?}. Retrying join...",
                            room_id, err
                        );
                        if let Ok(joined) = ctx.client.join_room_by_id(room_id).await {
                            let _ = joined.send(content).await;
                            delivered = true;
                        }
                    } else {
                        delivered = true;
                    }
                } else if let Ok(joined) = ctx.client.join_room_by_id(room_id).await {
                    let _ = joined.send(content).await;
                    delivered = true;
                }
            }
        }

        // 4. Update last briefing date so it cannot re-trigger today
        let _ = ctx
            .services
            .settings
            .update_last_briefing_date(&owner_id, &today_local_str)
            .await;

        // 5. Broadcast SSE event
        if let Some(ref broadcaster) = ctx.broadcaster {
            broadcaster.send(SystemEvent::MorningBriefingSent {
                owner_id: owner_id.to_string(),
                delivered_at: today_local_str.clone(),
                summary: format!(
                    "{} tasks, {} habits, {} bills, {} holdings",
                    payload.summary.pending_tasks_count,
                    payload.summary.habits_completed_count,
                    payload.summary.upcoming_bills_count,
                    payload.summary.holdings_count
                ),
            });
        }

        info!(
            "Morning briefing completed for {} (matrix delivered: {})",
            owner_id, delivered
        );
    }
}

/// Starts the background scheduler checking every 30 seconds.
pub fn start_briefing_scheduler(ctx: ModuleContext) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        info!("Starting background morning briefing scheduler worker loop (30s interval)...");
        let mut interval = tokio::time::interval(StdDuration::from_secs(30));

        loop {
            interval.tick().await;
            run_briefing_tick(&ctx).await;
        }
    })
}
