//! Matrix Reaction Handler for Todo and Reminder items.
//!
//! Handles 4 specific reaction emojis:
//! - 👍️ (or 👍): Mark Done (completes task or reminder)
//! - 👎️ (or 👎): Delete (deletes task or cancels reminder)
//! - 😝: Delay +1 Day (postpones task due date or reminder trigger by 24h)
//! - 🎉: Celebrate Completion (completes item and replies with celebration)

use crate::db::DbPool;
use chrono::{Duration, Utc};
use matrix_sdk::{
    room::Room,
    ruma::{
        events::{
            reaction::{OriginalSyncReactionEvent, ReactionEventContent},
            relation::Annotation,
            room::message::RoomMessageEventContent,
        },
        EventId,
    },
    Client, RoomState,
};
use tracing::{error, info, warn};

use crate::bot::reaction_targets::{delete_target_by_entity, get_target, BotMessageTarget};
use crate::db::Database;
use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::reminder::repository::ReminderRepository;
use crate::modules::reminder::service::{self as reminder_service, ReminderError};
use crate::modules::todo::repository::TodoRepository;
use crate::modules::todo::service::{self, TodoError, TodoPatch};
use crate::utils::html_escape;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllowedReaction {
    Done,
    Delete,
    DelayOneDay,
    Celebrate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionOutcome {
    pub ack_emoji: &'static str,
    pub reply_html: String,
}

pub fn parse_reaction_emoji(raw: &str) -> Option<AllowedReaction> {
    let normalized: String = raw
        .chars()
        .filter(|&c| c != '\u{fe0f}' && !c.is_whitespace())
        .collect();

    match normalized.as_str() {
        "👍" => Some(AllowedReaction::Done),
        "👎" => Some(AllowedReaction::Delete),
        "😝" => Some(AllowedReaction::DelayOneDay),
        "🎉" => Some(AllowedReaction::Celebrate),
        _ => None,
    }
}

pub fn register_reaction_handler(
    client: &Client,
    db: Database,
    broadcaster: Option<EventBroadcaster>,
) {
    let bot_user_id = match client.user_id() {
        Some(id) => id.to_owned(),
        None => {
            error!("Matrix client has no user_id — cannot register reaction handler");
            return;
        }
    };

    client.add_event_handler(move |ev: OriginalSyncReactionEvent, room: Room| {
        let own_id = bot_user_id.clone();
        let db = db.clone();
        let broadcaster = broadcaster.clone();

        async move {
            if room.state() != RoomState::Joined || ev.sender == own_id {
                return;
            }

            let action = match parse_reaction_emoji(&ev.content.relates_to.key) {
                Some(a) => a,
                None => return,
            };

            let sender_owner_id = match crate::accounts::AccountRepository::resolve_or_provision(
                &db.pool,
                ev.sender.as_str(),
            )
            .await
            {
                Ok(oid) => oid,
                Err(err) => {
                    warn!(
                        "Failed resolving account for reaction sender {}: {:?}",
                        ev.sender, err
                    );
                    return;
                }
            };

            let target_event_id = &ev.content.relates_to.event_id;
            let target = match get_target(&db.pool, target_event_id.as_str()).await {
                Ok(Some(t)) => t,
                Ok(None) => {
                    match crate::bot::reaction_targets::resolve_untracked_target(
                        &room,
                        target_event_id.as_str(),
                        &db.pool,
                        &sender_owner_id,
                    )
                    .await
                    {
                        Some(t) => t,
                        None => {
                            info!(
                                "No reaction target found for event {} in room {}",
                                target_event_id,
                                room.room_id()
                            );
                            return;
                        }
                    }
                }
                Err(err) => {
                    error!(
                        "Error querying reaction target {}: {:?}",
                        target_event_id, err
                    );
                    return;
                }
            };

            if sender_owner_id != target.owner_id {
                warn!(
                    "Unauthorized reaction attempt on {} #{} from sender {} (owner: {})",
                    target.entity_type, target.entity_id, ev.sender, target.owner_id
                );
                return;
            }

            info!(
                "Processing reaction {:?} on {} #{} by owner {}",
                action, target.entity_type, target.entity_id, sender_owner_id
            );

            if let Some(outcome) =
                apply_reaction(&db.pool, &target, action, broadcaster.as_ref()).await
            {
                send_reaction(&room, target_event_id, outcome.ack_emoji).await;
                if let Some(reply_eid) = send_reply(&room, &outcome.reply_html).await {
                    if action != AllowedReaction::Delete {
                        let _ = crate::bot::reaction_targets::record_target(
                            &db.pool,
                            reply_eid.as_str(),
                            room.room_id().as_str(),
                            &target.entity_type,
                            target.entity_id,
                            &target.owner_id,
                            &target.title,
                        )
                        .await;
                    }
                }
            }
        }
    });
}

pub async fn apply_reaction(
    pool: &DbPool,
    target: &BotMessageTarget,
    action: AllowedReaction,
    broadcaster: Option<&EventBroadcaster>,
) -> Option<ReactionOutcome> {
    match action {
        AllowedReaction::Done => handle_done(pool, target, broadcaster).await,
        AllowedReaction::Delete => handle_delete(pool, target, broadcaster).await,
        AllowedReaction::DelayOneDay => handle_delay(pool, target, broadcaster).await,
        AllowedReaction::Celebrate => handle_celebrate(pool, target, broadcaster).await,
    }
}

async fn handle_done(
    pool: &DbPool,
    target: &BotMessageTarget,
    broadcaster: Option<&EventBroadcaster>,
) -> Option<ReactionOutcome> {
    match target.entity_type.as_str() {
        "todo" => match service::update(
            pool,
            broadcaster,
            &target.owner_id,
            target.entity_id,
            TodoPatch {
                completed: Some(true),
                ..Default::default()
            },
        )
        .await
        {
            Ok(_) => Some(ReactionOutcome {
                ack_emoji: "✅",
                reply_html: format!(
                    "✅ Marked task as done: <i>\"{}\"</i>",
                    html_escape(&target.title)
                ),
            }),
            Err(TodoError::NotFound) => None,
            Err(err) => {
                error!("Failed marking todo #{} done: {:?}", target.entity_id, err);
                None
            }
        },
        "reminder" => match reminder_service::mark_triggered(
            pool,
            broadcaster,
            &target.owner_id,
            target.entity_id,
        )
        .await
        {
            Ok(_) => Some(ReactionOutcome {
                ack_emoji: "✅",
                reply_html: format!(
                    "✅ Marked reminder as done: <i>\"{}\"</i>",
                    html_escape(&target.title)
                ),
            }),
            Err(ReminderError::NotFound) => None,
            Err(err) => {
                error!(
                    "Failed marking reminder #{} done: {:?}",
                    target.entity_id, err
                );
                None
            }
        },
        _ => None,
    }
}

async fn handle_delete(
    pool: &DbPool,
    target: &BotMessageTarget,
    broadcaster: Option<&EventBroadcaster>,
) -> Option<ReactionOutcome> {
    match target.entity_type.as_str() {
        "todo" => {
            match service::delete(pool, broadcaster, &target.owner_id, target.entity_id).await {
                Ok(_) => {
                    let _ = delete_target_by_entity(pool, "todo", target.entity_id).await;
                    Some(ReactionOutcome {
                        ack_emoji: "🗑️",
                        reply_html: format!(
                            "🗑️ Deleted task: <i>\"{}\"</i>",
                            html_escape(&target.title)
                        ),
                    })
                }
                Err(TodoError::NotFound) => None,
                Err(err) => {
                    error!("Failed deleting todo #{}: {:?}", target.entity_id, err);
                    None
                }
            }
        }
        "reminder" => {
            // Deliberately not `reminder_service::cancel`: that mirrors
            // `ReminderRepository::cancel`, which only touches a *pending*
            // row. A 👎 here must still succeed on a reminder a prior 👍/🎉
            // already marked `triggered` — the reaction is "make the bot
            // message's item gone", not "cancel an active plan" — so this
            // stays a direct, unconditional status flip. See the Part A
            // report for the full reasoning.
            let res = crate::db_execute!(
                pool,
                "UPDATE reminders SET status = 'cancelled' WHERE id = ? AND owner_id = ?",
                target.entity_id,
                &target.owner_id
            );

            match res {
                Ok(r) if r > 0 => {
                    let _ = delete_target_by_entity(pool, "reminder", target.entity_id).await;
                    if let Some(bc) = broadcaster {
                        bc.send(SystemEvent::ReminderCancelled {
                            id: target.entity_id,
                            room_id: target.room_id.clone(),
                            owner_id: target.owner_id.clone(),
                        });
                    }
                    Some(ReactionOutcome {
                        ack_emoji: "🗑️",
                        reply_html: format!(
                            "🗑️ Deleted reminder: <i>\"{}\"</i>",
                            html_escape(&target.title)
                        ),
                    })
                }
                Ok(_) => None,
                Err(err) => {
                    error!(
                        "Failed cancelling reminder #{}: {:?}",
                        target.entity_id, err
                    );
                    None
                }
            }
        }
        _ => None,
    }
}

async fn handle_delay(
    pool: &DbPool,
    target: &BotMessageTarget,
    broadcaster: Option<&EventBroadcaster>,
) -> Option<ReactionOutcome> {
    match target.entity_type.as_str() {
        "todo" => {
            match TodoRepository::find_owned(pool, target.entity_id, &target.owner_id).await {
                Ok(Some(item)) => {
                    let new_due = item.due_at.unwrap_or_else(Utc::now) + Duration::days(1);
                    match service::update(
                        pool,
                        broadcaster,
                        &target.owner_id,
                        target.entity_id,
                        TodoPatch {
                            due_at: Some(Some(new_due)),
                            ..Default::default()
                        },
                    )
                    .await
                    {
                        Ok(_) => Some(ReactionOutcome {
                            ack_emoji: "⏰",
                            reply_html: format!(
                                "⏰ Postponed task by 1 day: <i>\"{}\"</i>",
                                html_escape(&target.title)
                            ),
                        }),
                        Err(TodoError::NotFound) => None,
                        Err(err) => {
                            error!(
                                "Failed updating todo #{} due date: {:?}",
                                target.entity_id, err
                            );
                            None
                        }
                    }
                }
                Ok(None) => None,
                Err(err) => {
                    error!("Failed fetching todo #{}: {:?}", target.entity_id, err);
                    None
                }
            }
        }
        "reminder" => {
            match ReminderRepository::find_owned(pool, &target.owner_id, target.entity_id).await {
                Ok(Some(rem)) => {
                    let new_trigger = rem.trigger_at + Duration::days(1);
                    match reminder_service::reschedule(
                        pool,
                        broadcaster,
                        &target.owner_id,
                        target.entity_id,
                        new_trigger,
                    )
                    .await
                    {
                        Ok(_) => Some(ReactionOutcome {
                            ack_emoji: "⏰",
                            reply_html: format!(
                                "⏰ Postponed reminder by 1 day: <i>\"{}\"</i>",
                                html_escape(&target.title)
                            ),
                        }),
                        Err(ReminderError::NotFound) => None,
                        Err(err) => {
                            error!(
                                "Failed rescheduling reminder #{}: {:?}",
                                target.entity_id, err
                            );
                            None
                        }
                    }
                }
                Ok(None) => None,
                Err(err) => {
                    error!("Failed fetching reminder #{}: {:?}", target.entity_id, err);
                    None
                }
            }
        }
        _ => None,
    }
}

async fn handle_celebrate(
    pool: &DbPool,
    target: &BotMessageTarget,
    broadcaster: Option<&EventBroadcaster>,
) -> Option<ReactionOutcome> {
    match target.entity_type.as_str() {
        "todo" => match service::update(
            pool,
            broadcaster,
            &target.owner_id,
            target.entity_id,
            TodoPatch {
                completed: Some(true),
                ..Default::default()
            },
        )
        .await
        {
            Ok(_) => Some(ReactionOutcome {
                ack_emoji: "🎉",
                reply_html: format!(
                    "🎉 Awesome work! Completed task: <i>\"{}\"</i> 🚀 Keep up the momentum!",
                    html_escape(&target.title)
                ),
            }),
            Err(TodoError::NotFound) => None,
            Err(err) => {
                error!("Failed marking todo #{} done: {:?}", target.entity_id, err);
                None
            }
        },
        "reminder" => match reminder_service::mark_triggered(
            pool,
            broadcaster,
            &target.owner_id,
            target.entity_id,
        )
        .await
        {
            Ok(_) => Some(ReactionOutcome {
                ack_emoji: "🎉",
                reply_html: format!(
                    "🎉 Fantastic! Completed reminder: <i>\"{}\"</i> 🚀",
                    html_escape(&target.title)
                ),
            }),
            Err(ReminderError::NotFound) => None,
            Err(err) => {
                error!(
                    "Failed marking reminder #{} done: {:?}",
                    target.entity_id, err
                );
                None
            }
        },
        _ => None,
    }
}

async fn send_reaction(room: &Room, target_event_id: &EventId, emoji: &str) {
    let rel = Annotation::new(target_event_id.to_owned(), emoji.to_string());
    let content = ReactionEventContent::new(rel);
    if let Err(err) = room.send(content).await {
        warn!(
            "Failed sending reaction {} to event {}: {:?}",
            emoji, target_event_id, err
        );
    }
}

async fn send_reply(room: &Room, html: &str) -> Option<matrix_sdk::ruma::OwnedEventId> {
    let plain = crate::utils::strip_html(html);
    let content = RoomMessageEventContent::text_html(&plain, html);
    match room.send(content).await {
        Ok(resp) => Some(resp.event_id),
        Err(err) => {
            error!("Failed sending reply to room {}: {:?}", room.room_id(), err);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::modules::reminder::models::{RecurrencePattern, ReminderSource};
    use crate::modules::todo::models::TodoPriority;

    async fn setup_pool() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        run_migrations(&pool).await.unwrap();
        pool
    }

    #[test]
    fn test_parse_reaction_emoji() {
        assert_eq!(parse_reaction_emoji("👍️"), Some(AllowedReaction::Done));
        assert_eq!(parse_reaction_emoji("👎️"), Some(AllowedReaction::Delete));
        assert_eq!(parse_reaction_emoji("👍"), Some(AllowedReaction::Done));
        assert_eq!(parse_reaction_emoji("👎"), Some(AllowedReaction::Delete));
        assert_eq!(
            parse_reaction_emoji("😝"),
            Some(AllowedReaction::DelayOneDay)
        );
        assert_eq!(parse_reaction_emoji("🎉"), Some(AllowedReaction::Celebrate));

        assert_eq!(parse_reaction_emoji("❤️"), None);
        assert_eq!(parse_reaction_emoji("🔥"), None);
    }

    #[tokio::test]
    async fn test_reaction_actions_on_todo() {
        let pool = setup_pool().await;

        let todo = TodoRepository::add(
            &pool,
            "!room:x",
            "@alice:x",
            "alice",
            "Prepare presentation",
            TodoPriority::High,
            Some(Utc::now()),
            None,
        )
        .await
        .unwrap();

        let target = BotMessageTarget {
            event_id: "$ev1".to_string(),
            room_id: "!room:x".to_string(),
            entity_type: "todo".to_string(),
            entity_id: todo.id,
            owner_id: "@alice:x".to_string(),
            title: todo.text.clone(),
        };

        // 1. Delay One Day (😝)
        let delay_res = apply_reaction(&pool, &target, AllowedReaction::DelayOneDay, None)
            .await
            .unwrap();
        assert_eq!(delay_res.ack_emoji, "⏰");
        assert!(delay_res.reply_html.contains("Postponed task by 1 day"));

        let item_after_delay = TodoRepository::find(&pool, todo.id).await.unwrap().unwrap();
        assert!(item_after_delay.due_at.unwrap() > todo.due_at.unwrap());

        // 2. Mark Done (👍)
        let done_res = apply_reaction(&pool, &target, AllowedReaction::Done, None)
            .await
            .unwrap();
        assert_eq!(done_res.ack_emoji, "✅");
        assert!(done_res.reply_html.contains("Marked task as done"));

        let item_after_done = TodoRepository::find(&pool, todo.id).await.unwrap().unwrap();
        assert!(item_after_done.completed);

        // 3. Celebrate (🎉)
        let celebrate_res = apply_reaction(&pool, &target, AllowedReaction::Celebrate, None)
            .await
            .unwrap();
        assert_eq!(celebrate_res.ack_emoji, "🎉");
        assert!(celebrate_res.reply_html.contains("Awesome work!"));

        // 4. Delete (👎)
        let delete_res = apply_reaction(&pool, &target, AllowedReaction::Delete, None)
            .await
            .unwrap();
        assert_eq!(delete_res.ack_emoji, "🗑️");
        assert!(delete_res.reply_html.contains("Deleted task"));

        let item_after_delete = TodoRepository::find(&pool, todo.id).await.unwrap();
        assert!(item_after_delete.is_none());
    }

    /// Completing a task by reaction used to leave its lead-time reminder
    /// pending, so it fired later for a task that was already done. The
    /// reaction path now goes through the same service the other transports
    /// use, which tears the reminder down as part of the same write.
    #[tokio::test]
    async fn test_completing_by_reaction_cancels_the_reminder() {
        let pool = setup_pool().await;

        let due = Utc::now() + Duration::hours(4);
        let todo = TodoRepository::add(
            &pool,
            "!room:x",
            "@alice:x",
            "alice",
            "Ship the thing",
            TodoPriority::High,
            Some(due),
            Some(60),
        )
        .await
        .unwrap();

        crate::modules::todo::due_reminder::sync(&pool, None, &todo, "!room:x")
            .await
            .unwrap();
        assert!(
            ReminderRepository::find_pending_for_source(&pool, ReminderSource::Todo, todo.id)
                .await
                .unwrap()
                .is_some(),
            "a reminder must be pending before the task is completed"
        );

        let target = BotMessageTarget {
            event_id: "$ev3".to_string(),
            room_id: "!room:x".to_string(),
            entity_type: "todo".to_string(),
            entity_id: todo.id,
            owner_id: "@alice:x".to_string(),
            title: todo.text.clone(),
        };

        let outcome = apply_reaction(&pool, &target, AllowedReaction::Done, None)
            .await
            .unwrap();
        assert_eq!(outcome.ack_emoji, "✅");

        assert!(
            ReminderRepository::find_pending_for_source(&pool, ReminderSource::Todo, todo.id)
                .await
                .unwrap()
                .is_none(),
            "completing the task by reaction must cancel its reminder"
        );
    }

    #[tokio::test]
    async fn test_reaction_actions_on_reminder() {
        let pool = setup_pool().await;

        let initial_trigger = Utc::now() + Duration::hours(2);
        let rem = ReminderRepository::create(
            &pool,
            "!room:x",
            "@bob:x",
            "bob",
            "Call dentist",
            initial_trigger,
            RecurrencePattern::None,
            None,
        )
        .await
        .unwrap();

        let target = BotMessageTarget {
            event_id: "$ev2".to_string(),
            room_id: "!room:x".to_string(),
            entity_type: "reminder".to_string(),
            entity_id: rem.id,
            owner_id: "@bob:x".to_string(),
            title: rem.message.clone(),
        };

        // 1. Delay One Day (😝)
        let delay_res = apply_reaction(&pool, &target, AllowedReaction::DelayOneDay, None)
            .await
            .unwrap();
        assert_eq!(delay_res.ack_emoji, "⏰");
        assert!(delay_res.reply_html.contains("Postponed reminder by 1 day"));

        let rem_after_delay = ReminderRepository::find_owned(&pool, "@bob:x", rem.id)
            .await
            .unwrap()
            .unwrap();
        assert!(rem_after_delay.trigger_at > initial_trigger);

        // 2. Mark Done (👍)
        let done_res = apply_reaction(&pool, &target, AllowedReaction::Done, None)
            .await
            .unwrap();
        assert_eq!(done_res.ack_emoji, "✅");
        assert!(done_res.reply_html.contains("Marked reminder as done"));

        let rem_after_done = ReminderRepository::find_owned(&pool, "@bob:x", rem.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rem_after_done.status, "triggered");

        // 3. Celebrate (🎉)
        let celebrate_res = apply_reaction(&pool, &target, AllowedReaction::Celebrate, None)
            .await
            .unwrap();
        assert_eq!(celebrate_res.ack_emoji, "🎉");
        assert!(celebrate_res.reply_html.contains("Fantastic!"));

        // 4. Delete / Cancel (👎)
        let delete_res = apply_reaction(&pool, &target, AllowedReaction::Delete, None)
            .await
            .unwrap();
        assert_eq!(delete_res.ack_emoji, "🗑️");
        assert!(delete_res.reply_html.contains("Deleted reminder"));

        let rem_after_delete = ReminderRepository::find_owned(&pool, "@bob:x", rem.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rem_after_delete.status, "cancelled");
    }
}
