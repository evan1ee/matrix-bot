# Quote-Based AI Confirmation Without Taint (TD-15) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Allow users in Matrix, Telegram, and other chat channels to confirm held AI actions by quoting the bot's own confirmation prompts without triggering untrusted input taint, while keeping webhook alerts, foreign messages, and cross-owner/cross-room quotes safely tainted.

**Architecture:** An in-memory bounded cache (`AiReplyTracker`) maintained inside `LlmService` tracks platform event IDs of authentic AI assistant replies along with their room and owner identity. Outbound AI replies are tagged with `is_ai` and `owner_id` on `Reply`, and recorded by channel adapters upon message transmission. Inbound quotes check `services.llm.is_authentic_reply(room_id, event_id, owner_id)`; authentic matches bypass taint, preventing infinite confirmation loops.

**Tech Stack:** Rust (async/tokio, std::sync::Mutex, std::collections::HashMap, std::time::{Instant, Duration}).

**Spec:** [`docs/superpowers/specs/2026-10-17-quote-based-ai-confirmation.md`](../specs/2026-10-17-quote-based-ai-confirmation.md)

## Global Constraints
- Maximum 500 production lines per file in `backend/src/` (excluding `#[cfg(test)]`).
- Layer dependencies run strictly one way (`domain` -> `application` -> `interface` / `llm` -> `infrastructure`), validated by `backend/tests/architecture.rs`.
- Zero SQL outside `infrastructure/postgres/repositories/` and tests.
- Fail-closed security: missing `event_id`, untracked messages, foreign accounts, or mismatched rooms unconditionally taint the turn.
- No new external crates; use `std::sync::Mutex`, `std::collections::HashMap`, and `Instant`.

## Review Focus
1. An untracked quote from `@bot:test` (such as an external webhook alert or daily briefing) must still taint the turn and refuse sensitive tools.
2. A quote with a valid event ID from Room A replayed in Room B must fail the room check and taint the turn.
3. A quote with a valid event ID generated for User A quoted by User B must fail the owner check and taint the turn.
4. An expired event ID (> 3600 seconds) must fail the TTL check and taint the turn.
5. In-place edited Matrix messages (`StageIndicator` with replacement event ID and placeholder event ID) must both be recognized regardless of which event ID the Matrix client quotes.

---

### Task 1: `AiReplyTracker` and `LlmService` Integration

**Files:**
- Create: `backend/src/application/llm/authentic_replies.rs`
- Modify: `backend/src/application/llm/mod.rs`
- Test: `backend/src/application/llm/authentic_replies.rs`

**Interfaces:**
- Consumes: `AccountId` from `crate::domain::accounts::AccountId`.
- Produces: `AiReplyTracker` struct with:
  - `pub fn new(ttl: Duration, max_capacity: usize) -> Self`
  - `pub fn record(&self, room_key: &str, event_id: &str, owner_id: AccountId)`
  - `pub fn is_authentic(&self, room_key: &str, event_id: &str, owner_id: &AccountId) -> bool`
  - `LlmService::record_authentic_reply(&self, room_key: &str, event_id: &str, owner_id: &AccountId)`
  - `LlmService::is_authentic_reply(&self, room_key: &str, event_id: &str, owner_id: &AccountId) -> bool`

- [ ] **Step 1: Write the failing unit tests for `AiReplyTracker` in `authentic_replies.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_record_and_validate_authentic_reply() {
        let tracker = AiReplyTracker::new(Duration::from_secs(60), 100);
        let owner = AccountId::new();
        tracker.record("!room1:server", "$event1", owner);

        assert!(tracker.is_authentic("!room1:server", "$event1", &owner));
        assert!(!tracker.is_authentic("!room2:server", "$event1", &owner)); // wrong room
        assert!(!tracker.is_authentic("!room1:server", "$event2", &owner)); // wrong event
        let other_owner = AccountId::new();
        assert!(!tracker.is_authentic("!room1:server", "$event1", &other_owner)); // wrong owner
    }

    #[test]
    fn test_expired_reply_is_not_authentic() {
        let tracker = AiReplyTracker::new(Duration::from_millis(10), 100);
        let owner = AccountId::new();
        tracker.record("!room1:server", "$event1", owner);
        std::thread::sleep(Duration::from_millis(20));
        assert!(!tracker.is_authentic("!room1:server", "$event1", &owner));
    }

    #[test]
    fn test_capacity_bounding() {
        let tracker = AiReplyTracker::new(Duration::from_secs(60), 2);
        let owner = AccountId::new();
        tracker.record("!room:server", "$ev1", owner);
        tracker.record("!room:server", "$ev2", owner);
        tracker.record("!room:server", "$ev3", owner);
        // $ev1 should be evicted when capacity of 2 is exceeded
        assert!(!tracker.is_authentic("!room:server", "$ev1", &owner));
        assert!(tracker.is_authentic("!room:server", "$ev2", &owner));
        assert!(tracker.is_authentic("!room:server", "$ev3", &owner));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p matrix-bot application::llm::authentic_replies`
Expected: FAIL (module or struct does not exist yet)

- [ ] **Step 3: Implement `AiReplyTracker` in `backend/src/application/llm/authentic_replies.rs` and attach to `LlmService`**

Implement `AiReplyTracker` with thread-safe `Arc<Mutex<HashMap<String, AuthenticReplyEntry>>>`, TTL expiration, and capacity eviction.
In `backend/src/application/llm/mod.rs`, add `pub authentic_replies: Arc<AiReplyTracker>` to `LlmService` and expose `record_authentic_reply` and `is_authentic_reply`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p matrix-bot application::llm::authentic_replies`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add backend/src/application/llm/
git commit -m "feat(llm): add AiReplyTracker for authentic reply identification"
```

---

### Task 2: `Reply` AI Provenance & Ownership Metadata

**Files:**
- Modify: `backend/src/interface/channels/message.rs:90-135`
- Modify: `backend/src/interface/channels/inbound.rs:110-125`

**Interfaces:**
- Consumes: `AccountId` from `crate::domain::accounts::AccountId`.
- Produces:
  - `Reply.is_ai: bool`
  - `Reply.owner_id: Option<AccountId>`
  - `Reply::ai(html: impl Into<String>) -> Self`
  - `Reply::mark_ai(mut self) -> Self`
  - `Reply::with_owner(mut self, owner: AccountId) -> Self`

- [ ] **Step 1: Write test for `Reply` constructors and builder methods**

In `backend/src/interface/channels/message.rs` test module:
Verify `Reply::html` defaults to `is_ai: false`, `Reply::ai` has `is_ai: true`, and `.with_owner` sets `owner_id`.

- [ ] **Step 2: Run test to verify failure**

Run: `cargo test -p matrix-bot interface::channels::message`
Expected: FAIL (fields/methods not defined)

- [ ] **Step 3: Implement fields and helpers on `Reply` and assign owner in `Inbound::handle`**

In `backend/src/interface/channels/message.rs`:
Add `pub is_ai: bool` and `pub owner_id: Option<AccountId>` to `Reply`.
Add `Reply::ai`, `Reply::mark_ai`, `Reply::with_owner`.
In `backend/src/interface/channels/inbound.rs`:
When `self.modules.dispatch` returns `Some(mut reply)`, ensure `if reply.owner_id.is_none() { reply.owner_id = Some(owner); }`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p matrix-bot interface::channels::message`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add backend/src/interface/channels/message.rs backend/src/interface/channels/inbound.rs
git commit -m "feat(channels): add is_ai flag and owner_id provenance to Reply"
```

---

### Task 3: Selective Taint Logic & Channel Transmission Recording

**Files:**
- Modify: `backend/src/interface/chat/llm.rs:90-105, 190-205`
- Modify: `backend/src/interface/channels/matrix/inbound.rs:225-270`
- Modify: `backend/src/interface/channels/telegram/worker.rs:170-190`

**Interfaces:**
- Consumes: `services.llm.is_authentic_reply`, `services.llm.record_authentic_reply`.
- Produces:
  - `LlmModule` emits `Reply::ai(html).with_owner(ctx.owner_id)`.
  - Quoted replies check `services.llm.is_authentic_reply(room_id, eid, owner_id)`.
  - Matrix `show_reply` records `sent_eid` and `ph_eid` if `reply.is_ai`.
  - Telegram `run_turn` records sent message ID if `reply.is_ai`.

- [ ] **Step 1: Update `LlmModule::handle_command` and `handle_ai_command_with_stage`**

In `backend/src/interface/chat/llm.rs`:
1. `LlmModule::handle_command` maps the returned reply HTML with `Reply::ai(html).with_owner(ctx.owner_id)`.
2. In `handle_ai_command_with_stage`:
```rust
    let mut turn_ctx = LlmTurnContext::new(services, pool, ai, owner_id, room_id, progress);
    if let Some(q) = quoted {
        let is_authentic = q
            .event_id
            .as_deref()
            .is_some_and(|eid| services.llm.is_authentic_reply(room_id, eid, owner_id));
        if !is_authentic {
            turn_ctx = turn_ctx.with_untrusted_input();
        }
    }
```

- [ ] **Step 2: Update Matrix `show_reply` to record authentic AI reply event IDs**

In `backend/src/interface/channels/matrix/inbound.rs`:
```rust
    if reply.is_ai {
        if let Some(ref owner) = reply.owner_id {
            let room_key = room.room_id().as_str();
            if let Some(ref eid) = sent_eid {
                services.llm.record_authentic_reply(room_key, eid.as_str(), owner);
            }
            if let Some(ref eid) = ph_eid {
                services.llm.record_authentic_reply(room_key, eid.as_str(), owner);
            }
        }
    }
```

- [ ] **Step 3: Update Telegram `worker.rs` to record authentic AI reply message IDs**

In `backend/src/interface/channels/telegram/worker.rs`:
When `outcome` is `InboundOutcome::Replied(ref reply)` and `reply.is_ai`, after successful delivery, record the sent message id in `services.llm.record_authentic_reply`.

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p matrix-bot`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add backend/src/interface/chat/llm.rs backend/src/interface/channels/matrix/inbound.rs backend/src/interface/channels/telegram/worker.rs
git commit -m "feat(chat): bypass taint on authentic AI replies and record sent AI message IDs"
```

---

### Task 4: Integration Tests for Quote-Based AI Confirmation

**Files:**
- Modify: `backend/src/interface/chat/llm.rs:460-530`

**Interfaces:**
- Consumes: `pool_with_todo`, `scripted_replies`, `ask`, `services.llm.record_authentic_reply`.
- Produces: Regression and confirmation tests proving that:
  - Quoting an authentic AI reply ("Delete todo #1? Please confirm.") executes `delete_todo` without confirmation required.
  - Quoting a webhook alert or foreign message still taints and refuses `delete_todo`.
  - Quoting an authentic reply with mismatched room or owner still taints.

- [ ] **Step 1: Write integration tests in `backend/src/interface/chat/llm.rs`**

```rust
#[tokio::test]
async fn quoting_an_authentic_ai_reply_does_not_taint_and_runs_the_tool() {
    use crate::llm::test_support::*;
    let (pool, owner, todo_id) = pool_with_todo("chat-quote-authentic").await;
    let services = crate::application::AppServices::new(&pool);
    let (url, _seen) = scripted_replies(vec![
        tool_reply("delete_todo", serde_json::json!({"id": todo_id})),
        text_reply("Deleted."),
    ])
    .await;

    // Record the authentic bot prompt event
    let event_id = "$bot_prompt_event_123:server";
    let room_id = "!room:mbm.one";
    services.llm.record_authentic_reply(room_id, event_id, &owner);

    let mut prompt_quote = quote("@bot:mbm.one", "Delete todo #1? Please confirm.");
    prompt_quote.event_id = Some(event_id.to_string());

    ask(
        &services,
        &pool,
        &url,
        &owner,
        "yes",
        Some(&prompt_quote),
    )
    .await;

    assert!(services
        .todos
        .find_owned(&owner, todo_id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn quoting_authentic_reply_with_wrong_room_or_owner_still_taints() {
    use crate::llm::test_support::*;
    let (pool, owner, todo_id) = pool_with_todo("chat-quote-wrong-context").await;
    let services = crate::application::AppServices::new(&pool);
    let (url, _seen) = scripted_replies(vec![
        tool_reply("delete_todo", serde_json::json!({"id": todo_id})),
        text_reply("ok"),
    ])
    .await;

    // Record under a different room
    let event_id = "$bot_prompt_other_room:server";
    services.llm.record_authentic_reply("!other_room:server", event_id, &owner);

    let mut quote_wrong_room = quote("@bot:mbm.one", "Delete todo #1? Please confirm.");
    quote_wrong_room.event_id = Some(event_id.to_string());

    ask(
        &services,
        &pool,
        &url,
        &owner,
        "yes",
        Some(&quote_wrong_room),
    )
    .await;

    // Tool should have been refused because wrong room caused taint
    assert!(services
        .todos
        .find_owned(&owner, todo_id)
        .await
        .unwrap()
        .is_some());
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p matrix-bot interface::chat::llm::tests::quoting_an_authentic_ai_reply`
Expected: PASS

- [ ] **Step 3: Run all chat llm tests**

Run: `cargo test -p matrix-bot interface::chat::llm`
Expected: ALL PASS

- [ ] **Step 4: Commit**

```bash
git add backend/src/interface/chat/llm.rs
git commit -m "test(llm): verify quote-based confirmation on authentic AI replies"
```

---

### Task 5: Full Verification & Documentation Update

**Files:**
- Modify: `docs/superpowers/HANDOFF.md`
- Modify: `docs/superpowers/reviews/2026-10-10-tech-debt-audit.md`

- [ ] **Step 1: Run AGENTS.md §8 verification gates**

1. `cargo fmt --check`
2. `cargo clippy --workspace --all-targets`
3. `cargo test --workspace --tests`
4. `cargo test --test architecture`

- [ ] **Step 2: Update `HANDOFF.md` and tech debt review**

Mark TD-15 resolved in `HANDOFF.md` and `reviews/2026-10-10-tech-debt-audit.md`. Record ruling for authentic AI reply verification.

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/HANDOFF.md docs/superpowers/reviews/2026-10-10-tech-debt-audit.md
git commit -m "docs(superpowers): mark TD-15 authentic AI reply tracking resolved"
```
