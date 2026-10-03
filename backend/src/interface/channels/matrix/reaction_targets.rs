//! Repository for tracking Matrix message targets (todos, reminders)
//! that can be interacted with via reactions.

use crate::db::DbPool;
use crate::domain::accounts::AccountId;
pub use crate::domain::chat::BotMessageTarget;
pub use crate::infrastructure::postgres::repositories::reaction_targets::{
    copy_targets, delete_target_by_entity, get_target, record_target,
};

/// Parses entity type, ID, and title from a bot confirmation HTML reply.
pub fn parse_entity_from_reply(reply_html: &str) -> Option<(&'static str, i64, String)> {
    // 1. Todo: "Added Task <b>#(\d+)</b> for .*: <i>\"(.*?)\"</i>"
    if let Some(pos) = reply_html.find("Added Task <b>#") {
        let after_hash = &reply_html[pos + "Added Task <b>#".len()..];
        if let Some(end_b) = after_hash.find("</b>") {
            if let Ok(id) = after_hash[..end_b].trim().parse::<i64>() {
                let title = extract_title_between_quotes(after_hash);
                return Some(("todo", id, title));
            }
        }
    }

    // 2. Reminder: "Reminder #(\d+) set!.*for: <i>\"(.*?)\"</i>"
    if let Some(pos) = reply_html.find("Reminder #") {
        let after_hash = &reply_html[pos + "Reminder #".len()..];
        if let Some(end_b) = after_hash.find(" set!</b>") {
            if let Ok(id) = after_hash[..end_b].trim().parse::<i64>() {
                let title = extract_title_between_quotes(after_hash);
                return Some(("reminder", id, title));
            }
        }
    }

    None
}

fn extract_title_between_quotes(s: &str) -> String {
    if let Some(start) = s.find("<i>\"") {
        let after_start = &s[start + 4..];
        if let Some(end) = after_start.find("\"</i>") {
            return after_start[..end].to_string();
        }
    }
    String::new()
}

/// Records a message target created during an AI tool execution (such as `set_reminder` or `add_todo`).
pub async fn record_ai_tool_target(
    pool: &DbPool,
    event_id: Option<&str>,
    room_id: &str,
    owner_id: &AccountId,
    func_name: &str,
    result_str: &str,
) {
    let eid = match event_id {
        Some(id) if !id.is_empty() => id,
        _ => return,
    };
    if func_name == "set_reminder" {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(result_str) {
            if let (Some(id), Some(msg)) = (
                val.get("id").and_then(|v| v.as_i64()),
                val.get("message").and_then(|v| v.as_str()),
            ) {
                let _ = record_target(pool, eid, room_id, "reminder", id, owner_id, msg).await;
            }
        }
    } else if func_name == "add_todo" {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(result_str) {
            let title = val
                .get("title")
                .or_else(|| val.get("text"))
                .and_then(|v| v.as_str());
            if let (Some(id), Some(t)) = (val.get("id").and_then(|v| v.as_i64()), title) {
                let _ = record_target(pool, eid, room_id, "todo", id, owner_id, t).await;
            }
        }
    }
}

/// Records the targets of every tool a turn ran, against the turn's progress
/// message (Matrix: the stage event), in execution order.
pub async fn record_ai_tool_targets(
    pool: &DbPool,
    event_id: Option<&str>,
    room_id: &str,
    owner_id: &AccountId,
    tool_results: &[(String, String)],
) {
    for (func_name, result_str) in tool_results {
        record_ai_tool_target(pool, event_id, room_id, owner_id, func_name, result_str).await;
    }
}

/// Links related events across a command turn (stage placeholder, final bot reply, user prompt event)
/// to ensure reactions on any of these messages resolve to the underlying entity.
pub async fn link_turn_targets(
    pool: &DbPool,
    stage_target_id: Option<&str>,
    reply_event_id: Option<&str>,
    user_event_id: &str,
) {
    if let (Some(ph_id), Some(reply_id)) = (stage_target_id, reply_event_id) {
        if ph_id != reply_id {
            let _ = copy_targets(pool, ph_id, reply_id).await;
        }
    }
    let source_id = reply_event_id.or(stage_target_id);
    if let Some(src) = source_id {
        let _ = copy_targets(pool, src, user_event_id).await;
    }
}

/// Dynamically resolves an untracked target message in the room by inspecting relations,
/// decrypting encrypted events if needed, and correlating with recent reminders or todos.
pub async fn resolve_untracked_target(
    room: &matrix_sdk::Room,
    target_event_id: &str,
    pool: &DbPool,
    owner_id: &AccountId,
) -> Option<BotMessageTarget> {
    use matrix_sdk::ruma::events::{
        room::message::{MessageType, SyncRoomMessageEvent},
        AnySyncMessageLikeEvent, AnySyncTimelineEvent,
    };
    use tracing::{info, warn};

    let eid = matrix_sdk::ruma::EventId::parse(target_event_id).ok()?;
    let timeline_event = match room.event(&eid, None).await {
        Ok(ev) => ev,
        Err(err) => {
            warn!(
                "resolve_untracked_target: failed fetching event {}: {:?}",
                target_event_id, err
            );
            return None;
        }
    };

    let any_event = match timeline_event.raw().deserialize() {
        Ok(AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomEncrypted(_))) => {
            // The match arm above just established this is `m.room.encrypted`;
            // ruma has no checked cast from the `Any*` enum to that one type.
            match room
                .decrypt_event(timeline_event.raw().cast_ref_unchecked(), None)
                .await
            {
                Ok(decrypted) => match decrypted.raw().deserialize() {
                    Ok(ev) => ev,
                    Err(err) => {
                        warn!(
                            "resolve_untracked_target: failed deserializing decrypted {}: {:?}",
                            target_event_id, err
                        );
                        return None;
                    }
                },
                Err(err) => {
                    warn!(
                        "resolve_untracked_target: failed decrypting {}: {:?}",
                        target_event_id, err
                    );
                    return None;
                }
            }
        }
        Ok(ev) => ev,
        Err(err) => {
            warn!(
                "resolve_untracked_target: failed deserializing event {}: {:?}",
                target_event_id, err
            );
            return None;
        }
    };

    let room_id_str = room.room_id().as_str();

    // 1. Check if the event is a replacement (edit) or rich reply relating to an already-tracked event
    let body = match any_event {
        AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(
            SyncRoomMessageEvent::Original(ref msg),
        )) => {
            if let Some(ref relates_to) = msg.content.relates_to {
                let parent_id = match relates_to {
                    matrix_sdk::ruma::events::room::message::Relation::Replacement(rep) => {
                        Some(rep.event_id.as_str())
                    }
                    matrix_sdk::ruma::events::room::message::Relation::Reply(reply) => {
                        Some(reply.in_reply_to.event_id.as_str())
                    }
                    _ => None,
                };
                if let Some(pid) = parent_id {
                    if let Ok(Some(parent_target)) = get_target(pool, pid).await {
                        let _ = record_target(
                            pool,
                            target_event_id,
                            room_id_str,
                            &parent_target.entity_type,
                            parent_target.entity_id,
                            &parent_target.owner_id,
                            &parent_target.title,
                        )
                        .await;
                        info!(
                            "Resolved untracked event {} via related parent target {}",
                            target_event_id, pid
                        );
                        return Some(parent_target);
                    }
                }
            }

            match msg.content.msgtype {
                MessageType::Text(ref t) => {
                    let f = t
                        .formatted
                        .as_ref()
                        .map(|x| x.body.as_str())
                        .unwrap_or_default();
                    format!("{}\n{}", t.body, f)
                }
                MessageType::Notice(ref n) => {
                    let f = n
                        .formatted
                        .as_ref()
                        .map(|x| x.body.as_str())
                        .unwrap_or_default();
                    format!("{}\n{}", n.body, f)
                }
                _ => return None,
            }
        }
        _ => return None,
    };

    let lower = body.to_lowercase();

    // 2. Reminder detection
    if lower.contains("reminder") || lower.contains("提醒") {
        if let Ok(mut reminders) =
            crate::infrastructure::postgres::repositories::reminders::list_user_reminders(
                pool, owner_id, None, None, 15, 0,
            )
            .await
        {
            reminders.sort_by_key(|r| {
                if r.status == crate::domain::reminders::ReminderStatus::Pending {
                    0
                } else {
                    1
                }
            });
            for rem in &reminders {
                if body.contains(&rem.message) || lower.contains(&rem.message.to_lowercase()) {
                    let _ = record_target(
                        pool,
                        target_event_id,
                        room_id_str,
                        "reminder",
                        rem.id,
                        owner_id,
                        &rem.message,
                    )
                    .await;
                    info!(
                        "Resolved untracked reminder #{} ({}) for event {}",
                        rem.id, rem.message, target_event_id
                    );
                    return Some(BotMessageTarget {
                        event_id: target_event_id.to_string(),
                        room_id: room_id_str.to_string(),
                        entity_type: "reminder".to_string(),
                        entity_id: rem.id,
                        owner_id: *owner_id,
                        title: rem.message.clone(),
                    });
                }
            }
            let in_room_pending: Vec<_> = reminders
                .iter()
                .filter(|r| {
                    r.room_id == room_id_str
                        && r.status == crate::domain::reminders::ReminderStatus::Pending
                })
                .collect();
            if in_room_pending.len() == 1 {
                let rem = in_room_pending[0];
                let _ = record_target(
                    pool,
                    target_event_id,
                    room_id_str,
                    "reminder",
                    rem.id,
                    owner_id,
                    &rem.message,
                )
                .await;
                info!(
                    "Resolved single pending reminder #{} for event {}",
                    rem.id, target_event_id
                );
                return Some(BotMessageTarget {
                    event_id: target_event_id.to_string(),
                    room_id: room_id_str.to_string(),
                    entity_type: "reminder".to_string(),
                    entity_id: rem.id,
                    owner_id: *owner_id,
                    title: rem.message.clone(),
                });
            }
        }
    }

    // 3. Todo task detection
    if lower.contains("task") || lower.contains("todo") || lower.contains("待办") {
        if let Ok(mut todos) = crate::infrastructure::postgres::repositories::todos::list_all_user(
            pool, owner_id, None, 15, 0,
        )
        .await
        {
            todos.sort_by_key(|t| if !t.completed { 0 } else { 1 });
            for todo in &todos {
                if body.contains(&todo.text) || lower.contains(&todo.text.to_lowercase()) {
                    let _ = record_target(
                        pool,
                        target_event_id,
                        room_id_str,
                        "todo",
                        todo.id,
                        owner_id,
                        &todo.text,
                    )
                    .await;
                    info!(
                        "Resolved untracked todo #{} ({}) for event {}",
                        todo.id, todo.text, target_event_id
                    );
                    return Some(BotMessageTarget {
                        event_id: target_event_id.to_string(),
                        room_id: room_id_str.to_string(),
                        entity_type: "todo".to_string(),
                        entity_id: todo.id,
                        owner_id: *owner_id,
                        title: todo.text.clone(),
                    });
                }
            }
            let in_room_open: Vec<_> = todos
                .iter()
                .filter(|t| t.room_id == room_id_str && !t.completed)
                .collect();
            if in_room_open.len() == 1 {
                let todo = in_room_open[0];
                let _ = record_target(
                    pool,
                    target_event_id,
                    room_id_str,
                    "todo",
                    todo.id,
                    owner_id,
                    &todo.text,
                )
                .await;
                info!(
                    "Resolved single open todo #{} for event {}",
                    todo.id, target_event_id
                );
                return Some(BotMessageTarget {
                    event_id: target_event_id.to_string(),
                    room_id: room_id_str.to_string(),
                    entity_type: "todo".to_string(),
                    entity_id: todo.id,
                    owner_id: *owner_id,
                    title: todo.text.clone(),
                });
            }
        }
    }

    None
}

/// Attempts to record a target if the reply message indicates an entity was created.
pub async fn try_record_from_reply(
    pool: &DbPool,
    event_id: &str,
    room_id: &str,
    owner_id: &AccountId,
    reply_html: &str,
) {
    if let Some((entity_type, entity_id, title)) = parse_entity_from_reply(reply_html) {
        let _ = record_target(
            pool,
            event_id,
            room_id,
            entity_type,
            entity_id,
            owner_id,
            &title,
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pool() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    async fn ensure_account(pool: &DbPool, owner: &AccountId) {
        crate::db::pool::seed_account(pool, owner).await;
    }

    #[test]
    fn test_parse_todo_reply() {
        let reply = "✅ Added Task <b>#42</b> for Alice: <i>\"Buy groceries\"</i> [Med]";
        let parsed = parse_entity_from_reply(reply);
        assert_eq!(parsed, Some(("todo", 42, "Buy groceries".to_string())));
    }

    #[test]
    fn test_parse_reminder_reply() {
        let reply = "⏱️ <b>Reminder #10 set!</b> I will remind you in <b>10 minutes</b> for: <i>\"Call doctor\"</i>";
        let parsed = parse_entity_from_reply(reply);
        assert_eq!(parsed, Some(("reminder", 10, "Call doctor".to_string())));
    }

    #[tokio::test]
    async fn test_crud_target() {
        let pool = setup_pool().await;
        let owner = AccountId::from_seed("alice");
        ensure_account(&pool, &owner).await;

        record_target(
            &pool,
            "$event123",
            "!room:x",
            "todo",
            1,
            &owner,
            "Test task",
        )
        .await
        .unwrap();

        let target = get_target(&pool, "$event123").await.unwrap().unwrap();
        assert_eq!(target.event_id, "$event123");
        assert_eq!(target.room_id, "!room:x");
        assert_eq!(target.entity_type, "todo");
        assert_eq!(target.entity_id, 1);
        assert_eq!(target.owner_id, owner);
        assert_eq!(target.title, "Test task");

        // Try update via record_target ON CONFLICT
        record_target(
            &pool,
            "$event123",
            "!room:x",
            "todo",
            1,
            &owner,
            "Updated task",
        )
        .await
        .unwrap();

        let target2 = get_target(&pool, "$event123").await.unwrap().unwrap();
        assert_eq!(target2.title, "Updated task");

        // Delete by entity
        let deleted = delete_target_by_entity(&pool, "todo", 1).await.unwrap();
        assert_eq!(deleted, 1);
        assert!(get_target(&pool, "$event123").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_record_ai_tool_target() {
        let pool = setup_pool().await;
        let owner = AccountId::from_seed("bob");
        ensure_account(&pool, &owner).await;

        // 1. AI creates reminder
        let rem_json = r#"{"id":12,"message":"Have my hair cut","room_id":"!room:x"}"#;
        record_ai_tool_target(
            &pool,
            Some("$rem_ev"),
            "!room:x",
            &owner,
            "set_reminder",
            rem_json,
        )
        .await;

        let rem_target = get_target(&pool, "$rem_ev").await.unwrap().unwrap();
        assert_eq!(rem_target.entity_type, "reminder");
        assert_eq!(rem_target.entity_id, 12);
        assert_eq!(rem_target.title, "Have my hair cut");
        assert_eq!(rem_target.owner_id, owner);

        // 2. AI creates todo
        let todo_json = r#"{"id":42,"title":"Buy groceries","room_id":"!room:x"}"#;
        record_ai_tool_target(
            &pool,
            Some("$todo_ev"),
            "!room:x",
            &owner,
            "add_todo",
            todo_json,
        )
        .await;

        let todo_target = get_target(&pool, "$todo_ev").await.unwrap().unwrap();
        assert_eq!(todo_target.entity_type, "todo");
        assert_eq!(todo_target.entity_id, 42);
        assert_eq!(todo_target.title, "Buy groceries");
        assert_eq!(todo_target.owner_id, owner);
    }

    #[tokio::test]
    async fn test_copy_targets_and_link_turn() {
        let pool = setup_pool().await;
        let owner = AccountId::from_seed("bob");
        ensure_account(&pool, &owner).await;

        record_target(
            &pool,
            "$placeholder",
            "!room:x",
            "reminder",
            12,
            &owner,
            "Have my hair cut",
        )
        .await
        .unwrap();

        // Copy to edit/reply event ID
        let copied = copy_targets(&pool, "$placeholder", "$edit_reply")
            .await
            .unwrap();
        assert_eq!(copied, 1);

        let edit_target = get_target(&pool, "$edit_reply").await.unwrap().unwrap();
        assert_eq!(edit_target.entity_type, "reminder");
        assert_eq!(edit_target.entity_id, 12);
        assert_eq!(edit_target.title, "Have my hair cut");
        assert_eq!(edit_target.owner_id, owner);

        // Link full turn (placeholder, final bot reply, user prompt event)
        link_turn_targets(
            &pool,
            Some("$placeholder"),
            Some("$edit_reply"),
            "$user_prompt",
        )
        .await;
        let user_target = get_target(&pool, "$user_prompt").await.unwrap().unwrap();
        assert_eq!(user_target.entity_type, "reminder");
        assert_eq!(user_target.entity_id, 12);
        assert_eq!(user_target.owner_id, owner);
    }
}
