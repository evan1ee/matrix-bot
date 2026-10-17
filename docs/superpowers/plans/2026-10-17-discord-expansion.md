# Discord Channel Capabilities Expansion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expand the Discord channel adapter with interactive ActionRow buttons for task completion, structured slash command options, and inbound receipt image ingestion over DMs.

**Architecture:** Extend Discord `render.rs` and `outbound.rs` to map `content.actions` into Discord `CreateActionRow` buttons, and handle `Interaction::Component` in `gateway.rs` and `worker.rs`. Enhance `commands.rs` to declare structured subcommands for `/todo` and `/expense` while retaining single-string `args` fallback. In `gateway.rs` and `worker.rs`, detect image attachments in direct messages, persist them to `StorageService`, and enqueue `media.process_receipt` background jobs.

**Tech Stack:** Rust (edition 2021), Serenity 0.12, Tokio, SQLx.

**Spec:** [`docs/superpowers/specs/2026-10-17-discord-expansion.md`](file:///D:/01_Data/code/matrix-hello-bot/docs/superpowers/specs/2026-10-17-discord-expansion.md)

## Global Constraints

- Maximum 500 lines per production file in `backend/src/` (excluding `#[cfg(test)]`).
- Follow layer boundaries: `interface/channels/discord/` must not import SQL or database repositories.
- Zero new external crates; use serenity 0.12 features and builders.
- Fail-closed security: receipt image processing restricted to direct messages and allowlisted/linked accounts; unlinked senders receive `UNLINKED_REPLY`.
- Every task ends with clean compilation, passing tests, and no warnings.

## Review Focus

1. **Button custom_id overflow:** If a button's `action.command` exceeds Discord's 100-byte `custom_id` limit, it must be dropped cleanly with a debug log rather than causing a Discord API rejection.
2. **Button quantity overflow:** If `content.actions` contains more than 25 buttons (Discord's 5x5 limit), it must be truncated at 25 without generating invalid component rows.
3. **Slash command options fallback:** Commands must cleanly parse both structured subcommands (e.g. `/todo add task:"milk"`) and legacy raw arguments (`/todo args:"add milk"`).
4. **Non-image media attachments:** Non-image attachments (e.g. `.txt`, `.pdf`, `.zip`) uploaded in DMs must not trigger receipt ingestion.
5. **Unlinked sender attachment:** An unlinked user sending an image in DMs must receive `UNLINKED_REPLY` and must not enqueue any jobs.

---

### Task 1: ActionRow Button Rendering & Component Interaction Parsing

**Files:**
- Modify: `backend/src/interface/channels/discord/render.rs`
- Modify: `backend/src/interface/channels/discord/outbound.rs`
- Modify: `backend/src/interface/channels/discord/inbound.rs`
- Test: unit tests in `render.rs`, `outbound.rs`, and `inbound.rs`

**Interfaces:**
- Consumes: `Content.actions` from `crate::interface::channels::message::Action`
- Produces:
  - `pub buttons: Vec<(String, String)>` on `Post` in `render.rs`.
  - `pub fn create_action_rows(buttons: &[(String, String)]) -> Vec<CreateActionRow>` in `outbound.rs`.
  - `pub fn from_component(interaction: &ComponentInteraction) -> Result<Parsed, CommandRefusal>` in `inbound.rs`.

- [ ] **Step 1: Write failing unit tests for button rendering and component parsing**

In `render.rs`:
```rust
#[test]
fn posts_render_actions_as_buttons() {
    let mut content = Content::text("Your tasks");
    content.actions = vec![Action {
        label: "Done #1".into(),
        command: "todo done 1".into(),
    }];
    let rendered = posts(&content).unwrap();
    assert_eq!(rendered[0].buttons, vec![("Done #1".into(), "todo done 1".into())]);
}
```
In `inbound.rs`:
Test that `from_component` parses custom_id into `InboundText::Command`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p matrix-bot interface::channels::discord::render`
Expected: FAIL (`buttons` field does not exist on `Post`).

- [ ] **Step 3: Implement button rendering in `render.rs`, rows in `outbound.rs`, and `from_component` in `inbound.rs`**

1. In `render.rs`:
   - Add `pub buttons: Vec<(String, String)>` to `Post`.
   - In `posts(content: &Content)`: map `content.actions` up to 25 items, checking custom_id fits within 100 bytes and label within 80 chars. Attach to the last post.
2. In `outbound.rs`:
   - Implement `create_action_rows(buttons: &[(String, String)]) -> Vec<CreateActionRow>`. Chunk by 5.
   - Attach components in `create_message`, `edit_response`, and `follow_up`.
3. In `inbound.rs`:
   - Implement `from_component(interaction: &ComponentInteraction) -> Result<Parsed, CommandRefusal>`.
   - Parse `interaction.data.custom_id` with `split_head` and `inbound_text`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p matrix-bot interface::channels::discord`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/src/interface/channels/discord/render.rs backend/src/interface/channels/discord/outbound.rs backend/src/interface/channels/discord/inbound.rs
git commit -m "feat(discord): render ActionRow buttons and parse component interactions"
```

---

### Task 2: Component Interaction Gateway & Worker Handling

**Files:**
- Modify: `backend/src/interface/channels/discord/gateway.rs`
- Modify: `backend/src/interface/channels/discord/worker.rs`
- Test: unit tests in `worker.rs`

**Interfaces:**
- Consumes: `from_component` from `inbound.rs`, `Work::Component`
- Produces:
  - Gateway reception of `Interaction::Component`.
  - Worker execution of button commands and editing the deferred response.

- [ ] **Step 1: Write failing unit test for `plan_command_reply` or component handling in `worker.rs`**

In `worker.rs`, add unit tests covering component reply planning and acknowledgment.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p matrix-bot interface::channels::discord::worker`
Expected: FAIL.

- [ ] **Step 3: Implement gateway component reception and worker execution**

1. In `gateway.rs`:
   - In `interaction_create`:
     - Match `Interaction::Component(component)`.
     - Parse via `from_component(&component)`.
     - Defer update or response.
     - Enqueue `Work::Component { interaction: Arc::new(component), parsed, mode }`.
2. In `worker.rs`:
   - Add `Work::Component` variant.
   - In `handle(work, inbound, http)`: handle `Work::Component` by executing `inbound.handle(&parsed.message, parsed.text, None)` and editing the response with `plan_command_reply`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p matrix-bot interface::channels::discord`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/src/interface/channels/discord/gateway.rs backend/src/interface/channels/discord/worker.rs
git commit -m "feat(discord): handle interactive button component interactions in gateway and workers"
```

---

### Task 3: Typed Slash Command Options Builders & Parser

**Files:**
- Modify: `backend/src/interface/channels/discord/commands.rs`
- Modify: `backend/src/interface/channels/discord/inbound.rs`
- Test: unit tests in `commands.rs` and `inbound.rs`

**Interfaces:**
- Consumes: `serenity::all::{CreateCommand, CreateCommandOption, CommandOptionType}`
- Produces:
  - Enhanced `create_commands()` advertising subcommands for `/todo` and `/expense`.
  - Subcommand parsing in `from_command` with fallback to `args`.

- [ ] **Step 1: Write failing unit tests for typed slash command parsing**

In `commands.rs` / `inbound.rs`:
Add tests for parsing `/todo add task:"milk"`, `/todo done id:5`, `/todo list`, and `/expense add amount:"15.50" note:"lunch"`, as well as legacy `/todo args:"add milk"`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p matrix-bot interface::channels::discord::commands`
Expected: FAIL.

- [ ] **Step 3: Implement typed subcommands in `commands.rs` and parsing in `inbound.rs`**

1. In `commands.rs`:
   - Update `create_commands()`:
     - For `todo`: add subcommands `add` (option `task: String`), `done` (option `id: Integer`), `list`, plus fallback option `args: String`.
     - For `expense`: add subcommands `add` (options `amount: String`, `note: String`), `list`, plus fallback option `args: String`.
2. In `inbound.rs`:
   - In `from_command`: check if options contain a subcommand (`CommandDataOptionValue::SubCommand`). If so, reconstruct normalized command arguments (`todo add <task>`, `todo done <id>`, etc.). If not, fall back to single string `args`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p matrix-bot interface::channels::discord::commands`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/src/interface/channels/discord/commands.rs backend/src/interface/channels/discord/inbound.rs
git commit -m "feat(discord): support typed slash command options for todo and expense"
```

---

### Task 4: Inbound Receipt Image Ingestion Over DMs

**Files:**
- Modify: `backend/src/interface/channels/discord/gateway.rs`
- Modify: `backend/src/interface/channels/discord/worker.rs`
- Modify: `backend/src/interface/channels/discord/inbound.rs`
- Test: unit tests in `inbound.rs` and `worker.rs`

**Interfaces:**
- Consumes: `AppServices.storage`, `NewJob::media_process_receipt`, `inbound.db()`, `StorageCategory::Receipts`
- Produces:
  - DM image attachment detection, download, receipt storage, and `media.process_receipt` job enqueuing.

- [ ] **Step 1: Write failing unit test for DM image attachment detection**

In `inbound.rs`, test image attachment filtering and metadata extraction.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p matrix-bot interface::channels::discord::inbound`
Expected: FAIL.

- [ ] **Step 3: Implement attachment detection and receipt job processing**

1. In `gateway.rs`:
   - In `message(&self, ctx: Context, message: Message)`:
     - If DM and not bot and has attachments:
       - If attachment has image mime or extension:
         - Enqueue `Work::DirectAttachment { channel, message_id, author, attachment, caption }`.
2. In `worker.rs`:
   - Handle `Work::DirectAttachment`:
     - Resolve user identity. If unlinked, reply with `UNLINKED_REPLY`.
     - If linked:
       - Download image bytes from Discord CDN (bounded by timeout, max 10 MiB).
       - Store image in `services.storage.put` with `StorageKey::generate(owner, StorageCategory::Receipts, ext)`.
       - Enqueue `media.process_receipt` job with `ConversationAddress::Discord(channel)`.
       - Reply with `🧾 Receipt received and queued for processing!`.

- [ ] **Step 4: Run tests and architecture verification**

Run:
```bash
cargo test -p matrix-bot interface::channels::discord
cargo test -p matrix-bot --test architecture
```
Expected: All PASS.

- [ ] **Step 5: Commit**

```bash
git add backend/src/interface/channels/discord/gateway.rs backend/src/interface/channels/discord/worker.rs backend/src/interface/channels/discord/inbound.rs
git commit -m "feat(discord): ingest DM receipt image attachments and enqueue media.process_receipt"
```

---

### Task 5: Documentation & Hand-Off Update

**Files:**
- Modify: `docs/superpowers/HANDOFF.md`

- [ ] **Step 1: Update Track 2 item 3 in `HANDOFF.md`**

Mark Track 2 item 3 as `(Fixed)` and summarize ActionRow buttons, typed slash commands, and DM receipt ingestion.

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/HANDOFF.md
git commit -m "docs(superpowers): resolve Discord channel capabilities expansion in handoff"
```
