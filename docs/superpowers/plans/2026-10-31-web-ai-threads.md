# Web AI Threads Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the web assistant keep several titled conversations, start a new one without deleting the old, rename threads, and copy / edit-and-resend messages.

**Architecture:** `ai_conversations` gets a nullable `title`. `LlmService` gains create / rename / truncate, publishing `AiConversationTitleUpdated` through the outbox. The first completed turn sets a heuristic title when none exists. The SPA lists threads, makes "New Chat" create an empty conversation (a turn without an id would otherwise resume the room's latest one for an hour), and Edit & Resend truncates from the edited message then sends.

**Tech Stack:** Rust/Axum/SQLx, utoipa, React 19 + React Query.

**Spec:** `docs/superpowers/specs/2026-10-31-feature-blueprints.md` — Blueprint 1.

## Global Constraints

- Title: trimmed, 1–100 characters, single line, no control characters (`ConversationTitle`).
- Prompt cap 4,000 characters at the boundary (existing `validate_prompt`).
- Cross-account conversation access → 404.
- Rulings: (1) **Edit & Resend truncates** (user decision, 2026-10-31): `parent_message_id` / branching is not built; (2) auto-title is the pure heuristic (word boundary, ≤ 50 chars) set when the first turn completes, not an LLM background job — no new job type, no extra provider call; (3) no `effective_model_name` field: `GET /account/ai-config` already returns the resolved model in `model_name`, so only the hardcoded `'gpt-4o-mini'` fallback in the SPA goes; (4) truncation is `POST /ai/conversations/{id}/truncate` with `{ "from_message_id" }` (this message and everything after it), called before the new prompt is sent.
- New events need an owner (`owner_id`) and are published inside the service transaction.

## Review Focus

- Renaming or truncating another account's conversation → 404, nothing changes.
- Title of 101 chars, empty, whitespace-only, or containing a newline → 400.
- Heuristic title from a long single word, multibyte (CJK/emoji) prompt, or markdown/URL prompt never splits a character and is never empty.
- Truncate with a message id from a different conversation → 404; truncating the first message empties the thread but keeps it.
- Second turn never overwrites a title the user set.

## Task 1: Title column, domain types, repository

**Files:** Create `backend/migrations/0015_ai_conversation_titles.sql`; modify `backend/src/domain/llm/` (add `ConversationTitle`, `title_from_prompt`; add `title: Option<String>` to `AiConversation`), `backend/src/infrastructure/postgres/repositories/llm.rs`, `backend/src/llm/models.rs`; `.sqlx/`.

**Interfaces:**
- Produces: `ConversationTitle::parse(&str) -> Result<ConversationTitle, LlmDomainError?>` (use the module's existing error type; maps to `AppError::Validation`), `ConversationTitle::as_str()`; `title_from_prompt(&str) -> Option<ConversationTitle>`; repo `create_conversation(conn, &AccountId, room_id: &str) -> Result<AiConversation, sqlx::Error>`, `set_conversation_title(conn, id: &str, &AccountId, &ConversationTitle, only_if_unset: bool) -> Result<bool, sqlx::Error>`, `truncate_messages_from(conn, conversation_id: &str, &AccountId, message_id: &str) -> Result<Option<u64>, sqlx::Error>` (`None` when the message is not in an owned conversation).

- [ ] Failing domain tests: bounds (0, 1, 100, 101 chars), newline/control rejection, trimming, `title_from_prompt` on long prompt, one 80-char word, CJK, emoji, whitespace-only (→ `None`).
- [ ] Failing repo tests: create is owner-scoped; `set_conversation_title` stranger → `false`, `only_if_unset` keeps an existing title; `truncate_messages_from` removes the message and later ones only, stranger / foreign message → `None`.
- [ ] Migration: `ALTER TABLE ai_conversations ADD COLUMN title TEXT; CREATE INDEX idx_ai_conversations_owner_updated ON ai_conversations (owner_id, updated_at DESC);`. Add `title` to every `AiConversation` SELECT.
- [ ] Regenerate `.sqlx/`, tests pass, commit `feat: conversation titles and truncate in the AI repository`.

## Task 2: Service, event, auto-title

**Files:** Modify `backend/src/application/llm/mod.rs`, `backend/src/domain/events.rs` (`AiConversationTitleUpdated { owner_id, conversation_id, title }`), `backend/src/llm/turn.rs` (set heuristic title after the assistant message is stored).

**Interfaces:** `LlmService::create_conversation(&self, &AccountId, room_id: &str) -> Result<AiConversation, AppError>`; `rename_conversation(&self, &AccountId, id: &str, &str) -> Result<AiConversation, AppError>` (404 when missing, event); `truncate_from(&self, &AccountId, conversation_id: &str, message_id: &str) -> Result<(), AppError>`; `auto_title(&self, &AccountId, conversation_id: &str, prompt: &str)` (no-op if titled, event when set).

- [ ] Failing tests: rename publishes the event and rejects invalid titles with `Validation`; stranger → `NotFound`; first turn sets the title and the second does not change it; user title survives a later turn; truncate stranger → `NotFound`.
- [ ] Implement; commit `feat: create, rename, truncate and auto-title AI conversations`.

## Task 3: REST + OpenAPI

**Files:** Modify `backend/src/interface/http/ai.rs`, `routes.rs`, `openapi.rs`, `backend/openapi.json`, `frontend/src/types/api.ts`.

**Interfaces:** `POST /ai/conversations` (body `{ "room_id"?: string }`) → 201 `AiConversation`; `PATCH /ai/conversations/{id}/title` `{ title }` → `AiConversation`; `POST /ai/conversations/{id}/truncate` `{ from_message_id }` → 204. `AiConversation` gains `title: string | null`.

- [ ] Failing handler tests: create → rename → bad titles 400 → truncate → stranger 404.
- [ ] Implement; regenerate OpenAPI + types; commit `feat: AI thread endpoints`.

## Task 4: Web UI

**Files:** Modify `frontend/src/modules/ai/AiAssistantView.tsx`, `MessageList.tsx`, `Bubbles.tsx`, `Composer.tsx` (only if needed), `useSystemEvents.ts` (invalidate `['ai-conversations']` on `AiConversationTitleUpdated`); create `frontend/src/modules/ai/ConversationSwitcher.tsx`; update `frontend/e2e/fake-api.ts`, `frontend/e2e/ai-chat.spec.ts` (+ new spec `ai-threads.spec.ts`).

- [ ] Failing Playwright specs: (a) new chat keeps the old thread listed and switching back shows its history; (b) title appears after the first turn; (c) rename; (d) badge shows the configured model, no hardcoded fallback; (e) copy button copies markdown; (f) Edit & Resend puts the text in the composer, truncates on send.
- [ ] Implement switcher popover (titles + relative times, `aria` listbox, Escape closes), non-destructive New Chat (POST create, becomes active; no-op when the active thread is empty), badge `config?.model_name ?? 'Configured model'`, per-message Copy and Edit & Resend on user messages (disabled while a turn is active).
- [ ] `npm --prefix frontend run build` and `test:e2e` (ai specs at minimum) pass; commit `feat: AI conversation switcher, new chat, copy and edit-and-resend`.

## Task 5: Gates and handoff

- [ ] fmt, clippy `-D warnings`, full backend suite (`--no-fail-fast -- --test-threads=2`), `cargo sqlx prepare --check`, openapi + generated-types drift check, frontend build, Playwright; update HANDOFF (status, rulings above) and `docs/api.md`.
