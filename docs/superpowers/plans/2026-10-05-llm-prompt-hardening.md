# Plan: LLM prompt hardening

Status: **implemented on branch `fix/llm-prompt-hardening`** (2026-10-06): D1-D5 as approved, plus decisions made during review: every quote taints the turn, including a quote of the bot's own message (D4 widened); an approved agent action runs at most once (started marker, a retry fails the run for a manual check); the model-step budget is `max_steps` plus one step per approved execution, capped at twice `max_steps`; `get_my_bookmarks` and OCR text are taint sources; the success-claim guard looks only at a reply that starts with the checkmark. See `docs/llm.md` and `docs/superpowers/HANDOFF.md` for the final rulings.
Roles: Opus plans and reviews; Sonnet implements and debugs. One commit per phase.

## Findings the audit missed

1. **Approved agent actions never run.** `approve_action` (`application/agent/approvals.rs:69`) sets the run back to `Executing` and re-queues it. Nothing runs the stored `tool_name`/`tool_input`; the engine asks the model again and a repeated sensitive call opens a new approval.
2. **The agent already sends `tools` natively and checks `tool_calls` first** (`engine.rs:128`), but rebuilds history as ReAct text, and on unparseable native arguments falls back to `{"arguments": raw}` (`engine.rs:131`), the same silent coercion as `parse_json_or_value`.
3. **Chat runs sensitive tools with no confirmation.** `delete_todo` and `sell_portfolio_holding` go straight through `turn.rs:228`; only the agent checks `sandbox::is_sensitive_tool`.

Also verified: no tool reads the owner from the prompt (`a_tool_cannot_act_for_another_account_named_in_its_arguments` proves executors ignore it); `web_chat` is already handled in code (`NO_ROOM_PLACEHOLDERS` in `domain/rooms`), so the prompt rule is redundant.

## Decisions (draft rulings)

- **D1: agent moves to native tool calling; ReAct removed.** `AgentStep` already stores structured calls, so each step maps to an assistant `tool_calls` message plus a `tool` message. No migration, no API change. Cost if wrong: a model without function calling cannot run agents; plain text becomes the final answer, so it fails safe.
- **D2: replace the zh/en templates with one rule for every language** ("after a successful action start with ✅ and a short bold title, then bullets of the key fields the tool returned, in the user's language"). Do not render replies in code. Cost if wrong: formatting drift; easy to revert.
- **D3: one language-independent success-claim guard** replacing the two English substring lists in `turn.rs` (~274–282, ~350–356): no tool succeeded this turn but the reply contains `✅`.
- **D4: taint rule for chat confirmation.** A sensitive tool is refused with a structured `confirmation_required` error when the turn carries quoted context, or an earlier round ran a tool returning outside content (`web_search`, `get_webhook_logs`, `knowledge_search`). The model asks the user; the user's next message performs it. Alternative: confirm every sensitive chat call (safer, more friction).
- **D5: dynamic context at the end of the single system message** (static prefix ~1.2k tokens, above the 1024 minimum). A custom `system_prompt` still replaces everything.

Known remaining cache breaks: conditional `web_search` tool changes the tools prefix; the minute-level time only affects the suffix.

## Phase 1: `fix(llm): split system prompt into stable prefix and short context` (findings 1, 4, 5, 6, 7, 10)

Files: `backend/src/llm/prompt.rs` (move footer fns to `llm/footer.rs` only if over 300 lines).

- 1.1 Pure renderer: `const SYSTEM_PROMPT_STATIC`, `PromptContext { timezone, now_local "YYYY-MM-DD HH:MM (Weekday)", currency, rooms, room_total, default_room }`, `render_system_prompt(&PromptContext)`. `build_system_prompt` only gathers data.
  Tests first: `static_prefix_is_byte_identical_across_contexts`; `context_has_minute_precision_and_no_utc`.
- 1.2 Remove the owner id from the output. Test `system_prompt_never_contains_owner_id` (`test_pool`).
- 1.3 Cap rooms at 20 plus "…and N more". Test `room_list_caps_at_twenty_and_counts_rest`; rooms beyond the cap still resolve by name.
- 1.4 Rewrite static text: sections, sentence case, each rule once; dependent calls must wait for earlier results (calls in one round run concurrently); date rules once; drop the `web_chat` line; quoted-reply intents reference `<quoted_message>`. Test `static_prompt_has_no_hardcoded_language_templates`.

Risk: wording changes alter model behaviour; review manually against the old prompt.

## Phase 2: `fix(llm): treat quoted and tool content as data; confirm tainted sensitive calls` (findings 3, 5)

Files: `domain/chat.rs`, `interface/channels/matrix/quote.rs`, `interface/chat/llm.rs`, `llm/context.rs`, `llm/turn.rs`, `llm/agent/sandbox.rs`.

- 2.1 `to_context_prompt` emits `<quoted_message author="…">…</quoted_message>` then `User request: …`; escape an embedded closing tag. Add the untrusted-content rule to the static prompt. Test `quoted_body_cannot_close_its_own_delimiter`.
- 2.2 `LlmTurnContext::with_quoted_context()`; track `tainted` in the turn loop and refuse sensitive tools while true. Tests: `sensitive_tool_from_quoted_turn_is_refused_with_confirmation_required`, `sensitive_tool_after_web_search_is_refused`, `sensitive_tool_in_plain_turn_executes`.
- 2.3 Track `any_tool_succeeded`; delete both substring lists. Tests: `checkmark_reply_without_successful_tool_triggers_one_correction`, `checkmark_reply_after_successful_tool_is_kept`.

Risk: if `turn.rs` passes ~450 production lines, extract `llm/tool_round.rs`.

## Phase 3: `refactor(agent)!: native tool calling for agent runs` (findings 2, 8, 9)

Files: `llm/agent/{prompts,engine,mod}.rs`, `interface/jobs/agent.rs`.

- 3.1 `build_messages(goal, steps, tools, step_no, max_steps)`: system prompt; `Goal:` user message; per step an assistant `tool_calls` message (id = step id) plus a `tool` message; a blocked step with no output becomes `{"status":"awaiting_user_approval"}`; final `Step k of N.` Tools go only through `tools`. Tests: `steps_render_as_native_tool_call_pairs`, `blocked_step_renders_awaiting_approval_observation`.
- 3.2 Accept only native `tool_calls` (first one). Invalid arguments give `StepOutcome::InvalidCall`, recorded as a `Failed` step with `{"error": …}`, run continues. Plain text is `Completed`; empty text is `Failed`. Delete `parse_react_response`, `ParsedReact`, `parse_json_or_value`, `extract_thought` and their tests. Tests: `malformed_native_arguments_never_execute_tool`, `text_reply_is_final_answer`, `run_continues_after_invalid_call`.
- 3.3 `format_tool_output` always returns valid JSON (arrays drop tail items plus `{"_truncated":true,"omitted":k}`; other values become `{"_truncated":true,"preview":…,"original_chars":n}`). Tests: `truncated_array_output_parses_and_counts_omitted`, `truncated_object_output_parses`.
- 3.4 Agent prompt: step budget via `StepContext.max_steps`; on tool error fix arguments or change approach, never repeat an identical failing call, stop after two failures; ambiguous goal gives a final answer asking the question; untrusted-data rule. Test `agent_prompt_states_step_budget`.

## Phase 4: `fix(agent): execute the approved action on resume`

Files: `application/agent/approvals.rs`, `domain/agent/step.rs`, `interface/jobs/agent.rs`.

`approve_action` records a marker step (`{"approved":true}`, `Success`) in its transaction, like `reject_action`. Add `AgentStep::is_approved_pending_execution()`. When the handler sees it last, it runs the stored call through `ToolRegistry` (no model call, no sensitivity check), records the result, then resumes. No new SQL. Tests: `approved_action_executes_exactly_once_on_resume`, `rejected_action_is_not_executed`.

Risk: a crash between running the tool and recording the result re-executes it on the next run; document in the design note.

## Phase 5: `docs: llm prompt hardening`

`docs/llm.md` (prompt structure and caching, untrusted-content rule, taint confirmation, native agent calling, approval resume) and `docs/superpowers/HANDOFF.md` (rulings D1–D5 in `Ruling: <decision> — <why> — <cost if wrong>` form; fix the "ReAct loop" line). Each phase also updates docs in its own commit.

Order: P1 then P2; P3 parallel to P1/P2; P4 after P3. No `openapi.json`, schema or `.sqlx` changes expected.

## Verification (every phase; no external services, never run the bot)

```bash
cargo fmt --all -- --check
cargo check -p matrix-bot --all-targets
cargo test  -p matrix-bot llm::
cargo test  -p matrix-bot --all-targets      # TEST_DATABASE_URL on a throwaway local PG
cargo clippy -p matrix-bot --all-targets -- -D warnings
cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json
git diff --check
```

Each regression test must be seen failing before its fix.

## Open decisions

1. Approve D1 (native tool calling, ReAct removed)?
2. D2: drop the exact zh/en templates?
3. D4: confirm only tainted sensitive calls, or every sensitive chat call?
4. Keep Phase 4 here or split into its own plan?
5. Append the Context section to a user's custom system prompt?

## Follow-ups (out of scope)

Chat tool outputs are not truncated; `RoomTarget::RoomId` is not checked against the owner's room cache; conditional `web_search` breaks the tools-prefix cache.

## Opus review checklist

- [ ] Static prefix has no interpolation, proven byte-identical; no seconds, UTC or owner id.
- [ ] Each rule once; no ALL-CAPS, zh/en templates or `web_chat` line; dependent-call rule present.
- [ ] Room cap tested; rooms beyond the cap resolve by name.
- [ ] Quote delimiter can't be closed from inside; untrusted-content rule in chat and agent prompts.
- [ ] Tainted sensitive call refused; plain-turn call runs.
- [ ] No path coerces bad arguments into an executing call (`grep '"query": trimmed\|"arguments": first'` empty).
- [ ] ReAct parsing deleted; tool_call ids pair; truncation yields valid JSON.
- [ ] Approved action runs once; rejected never runs.
- [ ] No file over 500 production lines; openapi diff empty; all gate output reported.
- [ ] `docs/llm.md` and HANDOFF updated in the same commits.
