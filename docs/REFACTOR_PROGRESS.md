# Architecture Review — Implementation Progress

Branch: `refactor/architecture-review`
Plan of record: [docs/ARCHITECTURE_REVIEW.md](ARCHITECTURE_REVIEW.md)

Workflow per item: Opus 5 plans → Sonnet 5 implements → Opus 5 verifies → fix before moving on.

## Baseline (commit `a731f20`)

| Check | Result |
| --- | --- |
| `cargo test --workspace` | 580 passed, **1 pre-existing failure**, 7 ignored |
| Pre-existing failure | `modules::ai::tools::tests::test_tool_definitions` — asserted 47 definitions, registry had 48 since `get_my_net_worth` landed in `1138ef9` |
| Action taken | Corrected the count in its own commit (`a731f20`) so "tests pass" is a meaningful signal for every item that follows |
| Green baseline | 581 passed, 0 failed, 7 ignored |

## Status

| Item | Scope | Planned | Implemented | Verified | Notes |
| --- | --- | --- | --- | --- | --- |
| R1a | AR-001 — ownership-table drift, classification, transactional merge | ✅ | ✅ | ✅ | `ae2ed29`. 587 pass. Guards mutation-tested. |
| R1b | AR-001 — `portfolio_audit_log` merge semantics | ✅ | ✅ | ✅ | `2f35a29`. 592 pass. Move + re-chain, per your decision. |
| R3 | AR-003 — stop modules depending on the HTTP layer | ✅ | ✅ | ✅ | `4893c04`. 593 pass. Guard proven to catch a compiling violation. |
| R2 | AR-004 — `Config` threaded into the bot side | ✅ | ✅ | ✅ | `5fdfdfa`. 594 pass. AI env reads deferred to R2b. |
| R2b | AR-004 — AI client env reads into Config | ✅ | ✅ | ✅ | `ccafcce`. 597 pass. `search.rs` deferred to R2c. |
| R4 | AR-006 — `BotHandles`, `ApiState::from_config` | ✅ | ✅ | ✅ | `f7e682b`. 596 pass. Gate is compiler-enforced. |
| R5 | AR-007 — collapse the portfolio facade | ✅ | ✅ | ✅ | `3b49628`. 596 pass, count unchanged. −293 lines. |
| R11 | AR-009 — credential resolution extracted | ✅ | ✅ | ✅ | `6633de7`. 597 pass. Found a real step-up hole. |
| R6 | AR-002 — todo service | ✅ | ✅ | ✅ | `7ffb79d`. 605 pass. Fixed 4 user-visible bugs. |
| R7 | AR-002 — finance service | ✅ | ✅ | ✅ | `72ed406`. Caught a regression in review. |
| R8 | AR-002 — portfolio service | ✅ | ✅ | ✅ | `922fccd`. 620 pass. |
| R9 | AR-002 — reminder service + AiError | ✅ | ✅ | ✅ | `daf016c`, `349768f`. 630 pass. |
| R10 | AR-005 — split `bot/handler.rs` | ✅ | ✅ | ✅ | `44283f1`. 634 pass. 487 → 102 lines. |
| R12a | AR-008 — OpenAPI coverage for the undocumented API files | ✅ | ✅ | ✅ | `e3f987a`. 635 pass. 87 → 107 paths. |
| R12b | AR-008 — generated DTOs | ✅ | ✅ | ✅ | `f938e30`. Found a live production bug. |
| R2c | AR-004 — `search.rs` env reads | — | — | — | **Declined**, see below |

## Findings that correct the review

### F-1 — `portfolio_audit_log` cannot simply be added to `OWNED_TABLES` (corrects AR-001)

AR-001 named `portfolio_audit_log` as one of three tables missing from `OWNED_TABLES` and implied a plain
addition. Two constraints discovered during planning make that wrong:

1. `backend/src/db/schema_portfolio.rs:89-109` defines `prevent_portfolio_audit_update` and
   `prevent_portfolio_audit_delete`, both `BEFORE` triggers that `RAISE(ABORT)`.
2. `backend/src/modules/portfolio/audit.rs:47-58` folds `owner_id` into `entry_hash`, and
   `verify_audit_chain` (`:197`) walks `WHERE owner_id = ? ORDER BY id ASC` expecting contiguous linkage
   from the genesis hash.

Measured directly against the real schema: a plain `UPDATE` **and** an `UPDATE OR IGNORE` both fail with
`(code: 1811) Audit log entries are immutable and cannot be updated`. The `OR IGNORE` conflict clause does
not suppress a trigger's `RAISE(ABORT)`.

So adding the table to `OWNED_TABLES` as the review described would not merely be incomplete — it would make
**every merge fail** for any account with portfolio activity. Moving those rows requires dropping the trigger
inside the transaction and recomputing the entire chain for the surviving owner, which is a real decision
about what a tamper-evident ledger means across a merge.

Split out as **R1b**. R1a names the table in a dedicated `OWNER_ID_UNMERGED` constant so the derived-set test
passes and the exception is explicit and tested rather than silently absent.

### F-2 — the `account_id` side was further along than the review stated (corrects AR-001)

`merge.rs:191-203` already re-points `security_events` as well as `matrix_links` and `webauthn_credentials`.
The genuinely unhandled `account_id` tables are `account_rooms`, `mcp_tokens`, `passkey_enrollment_tokens`,
`matrix_link_challenges`, and the legacy `web_users`.

### F-3 — the repository is not `rustfmt`-clean at the branch point (pre-existing)

`cargo fmt --all -- --check` reports 60 diffs across roughly 20 files at `1138ef9`, before any of this work.
They are real formatting differences (over-long lines that rustfmt would wrap), not line-ending noise. CI runs
that exact command, so this step was already failing on the branch.

Left alone deliberately: reformatting 20 unrelated files is exactly the drive-by change this workstream is
meant to avoid, and it would bury every subsequent diff. Each item here instead formats only the files it
touches, checked with `rustfmt --check` on those paths. Worth its own commit at some point, on its own.

## Completed items

### R1a — AR-001 ownership-table drift (`ae2ed29`)

**Changed:** `backend/src/accounts/mod.rs`, `backend/src/accounts/merge.rs`.

**Verified by Opus 5:**

| Check | Result |
| --- | --- |
| `cargo check --workspace --all-targets` | 0 warnings, exit 0 |
| `cargo test --workspace` | 587 passed, 0 failed, 7 ignored (baseline 581, net +6) |
| `rustfmt --check` on both changed files | clean |
| OpenAPI regenerated | no diff — `MergePreview` kept its fields, the two new tables fall into `other` |
| Public API | `MergeRepository::merge` signature unchanged; no caller touched |

**Mutation-tested the new guards** rather than trusting that they pass:

| Mutation | Caught? |
| --- | --- |
| Remove `finance_shares` from `OWNED_TABLES` | ✅ `these tables carry owner_id but no list classifies them: ["finance_shares"]` |
| Remove `mcp_tokens` from `ACCOUNT_ID_DISCARDED` | ✅ `these tables carry account_id but no list classifies them: ["mcp_tokens"]` |
| Add a table name that is not in the schema | ✅ `these tables are classified but are not in the schema: ["not_a_real_table"]` |

**Deviation from plan, accepted:** the pre-existing `test_no_owned_table_is_left_behind` inserts one row per
entry in `OWNED_TABLES` and asserts the total. Growing the constant from 20 to 22 required inserting rows for
the two new tables and raising the expected count. Reviewed as extending coverage in step with the constant,
not weakening it — the test still asserts one arrived row for every table including the new ones.

### R1b — AR-001 audit ledger across a merge (`2f35a29`)

Decision taken by the user from three options: **move the ledger and re-chain it**, rather than leaving it
behind as a tombstone or keeping the documented exception.

**Changed:** `modules/portfolio/audit.rs` (new `absorb_ledger`), `db/schema_portfolio.rs` (shared
`AUDIT_UPDATE_TRIGGER` constant), `accounts/mod.rs` (`OWNER_ID_UNMERGED` → `OWNER_ID_MERGED_SEPARATELY`),
`accounts/merge.rs`, `frontend/.../ActivitySection.tsx`, `frontend/.../types.ts`.

`portfolio_audit_log` stays out of `OWNED_TABLES` on purpose. Putting it there would send it through the
generic `UPDATE OR IGNORE` loop, which the immutability trigger aborts, breaking every merge.

**Verified by Opus 5:**

| Check | Result |
| --- | --- |
| `cargo check --workspace --all-targets` | 0 warnings |
| `cargo test --workspace` | 592 passed, 0 failed, 7 ignored (was 587) |
| `rustfmt --check` on all four changed `.rs` files | clean |
| `cargo run --bin generate-openapi` | no diff — `MergePreview` shape held |
| `npm --prefix frontend run build` | succeeded (pre-existing chunk-size warning only) |

**Mutation-tested the two guarantees:**

| Mutation | Caught? |
| --- | --- |
| Delete the trigger-recreation call | ✅ `test_the_immutability_trigger_survives_a_merge` fails |
| Neuter the chain-rebuild loop | ✅ `test_the_absorbed_ledger_still_verifies` fails |

A first mutation attempt silently failed to apply and the tests passed — which proves nothing. Re-run with
line-based edits, confirmed applied, and both then failed correctly. Worth repeating on later items: always
confirm the mutation landed before trusting a red or green result.

**Deviation from plan, accepted:** `frontend/src/modules/portfolio/types.ts` was not in the planned file list,
but `PortfolioAuditEntry.action` is a hand-written union there, so `case 'merge'` would not typecheck without
adding the variant. That file being hand-maintained rather than generated from `openapi.json` is exactly the
drift AR-008 describes, and is a good concrete example to cite when R12 comes round.

### R3 — AR-003 layer direction (`4893c04`)

**Changed:** day-activity collection moved to `modules/life/activity.rs` + `activity_collectors.rs`; room
discovery and authorisation moved to a new `modules/rooms/`; `api/life_activity.rs` and `api/rooms.rs` reduced
to their handlers; callers in `api/webhooks.rs` (3), `modules/journal/handler.rs` and
`modules/webhook/scheduler.rs` updated; guard test added to `modules/mod.rs`.

**Plan corrected during implementation.** AR-003 said to move the room functions into `db/rooms.rs`, quoting
`AGENTS.md`. Checked first: `backend/src/db/` has **zero** `matrix_sdk` imports, and both functions need a live
`matrix_sdk::Client`. Following the review literally would have made the persistence layer depend on the Matrix
SDK — a worse inversion than the one being fixed. They went to a new `modules/rooms/` instead. `AGENTS.md` §4
is right about the *cached* room data, which is already in `db/rooms.rs` and stayed there.

**Verified by Opus 5:**

| Check | Result |
| --- | --- |
| `cargo check --workspace --all-targets` | 0 warnings |
| `cargo test --workspace` | 593 passed, 0 failed, 7 ignored (was 592, +1 guard test) |
| `grep -rn "crate::api" backend/src/modules/` | no matches |
| Moved code byte-identical | ✅ for all five functions except two lines where `crate::accounts::AccountRepository` shortened to `AccountRepository` under the new import |
| `cargo run --bin generate-openapi` | no diff |
| `rustfmt --check` on every touched leaf file | clean |
| History | `git mv` used, so the collectors file keeps its history |

**Guard mutation-tested twice.** Reintroducing the old call path fails to *compile*, so it never reaches the
test — that proved nothing. Re-run with a violation that does compile (`use crate::api::auth::ApiState;` added
to `modules/todo/mod.rs`): the guard failed and named the file, and passed again once reverted.

**Bug found in the plan, fixed by the implementer:** the guard test as I specified it searched for a string
literal that appears in its own source, so it flagged itself and could never pass. It now builds the needle at
runtime with a comment explaining why. Worth remembering for any future source-scanning test.

**Deferred, noted:** `db/rooms.rs` still has four free functions that only delegate to the identical
`RoomRepository` methods. Real duplication, unrelated to dependency direction, left for a quick-win pass.

### R2 — AR-004 configuration threading, bot side (`5fdfdfa`)

**Changed:** `bot/mod.rs` (MatrixBot keeps its Config), `bot/handler.rs` (four env reads gone),
`bot/invite.rs` (allowlist passed in), `config.rs` (`database_url_from_env`, guard test), `main.rs`
(MCP requires `--owner`), `docs/MCP_SERVER.md`.

**Scope split.** The review's R2 also covered `modules/ai/{client,models,search}.rs`, which hold about
twenty env reads behind five-deep fallback chains plus a process-global `OnceLock` that is initialised
lazily. Moving those is a behaviour-sensitive change of its own, so it is **R2b**. Rather than postpone the
guard, the guard ships now with an explicit allowlist naming exactly what is not fixed, so no *new* env read
can appear anywhere while R2b is outstanding.

**Verified by Opus 5:**

| Check | Result |
| --- | --- |
| `cargo check --workspace --all-targets` | 0 warnings |
| `cargo test --workspace` | 594 passed, 0 failed, 7 ignored (was 593) |
| Behaviour preservation | every `Config` default compared against the inline default it replaced: `ocr_model_dir`, `voice_enabled` parse, `voice_model_dir`, `voice_language`, allowlist split — all identical |
| OCR `enabled` check | deliberately still absent, as before; adding it would have been a silent behaviour change |
| `cargo run --bin generate-openapi` | no diff |
| `rustfmt --check` on the four leaf files | clean |
| MCP change, run end to end | without `--owner`: exits with the new message. with `--owner acct_test`: starts normally |

**Guard mutation-tested.** First attempt inserted invalid Rust and only produced a compile error — no signal.
Re-run with a valid `fn probe_env() { std::env::var("PROBE") }` appended to `events.rs`: the guard failed,
naming `events.rs:331`, and passed again once removed.

**Accepted noise:** rustfmt reflowed three pre-existing `assert!` calls in the `config.rs` test module,
because formatting the file it was already editing covers the whole file. Left in place — reverting them would
leave `config.rs` not rustfmt-clean, which is the worse end state given F-3.

### R4 — AR-006 composition root (`f7e682b`)

**Changed:** new `bot/handles.rs`; `api/auth/types.rs` (`from_config`, `trust_forwarded_for`, test
constructors gated); `api/mod.rs` (`start_api_server` takes handles, returns `Result`); `bot/mod.rs`
(`new_full` takes `BotHandles`); `main.rs` (owns composition); `config.rs`; `api/auth/recovery.rs`.

**Scope trimmed from the review.** AR-006 also proposed grouping `ApiState`'s 15 fields into sub-structs.
Dropped: it touches every field access site across the API layer for no behavioural or safety gain. The two
parts that carried the risk — unsafe defaults and untyped wiring — are done.

**Cheaper approach than planned.** The review said to rename `new` to `for_tests`, which meant editing 39 call
sites. Marking `new` and the three builders `#[cfg(test)]` instead gives the same guarantee with 38 call sites
untouched, and makes it a compiler gate rather than a convention.

**Verified by Opus 5:**

| Check | Result |
| --- | --- |
| `cargo check --workspace --all-targets` | 0 warnings |
| `cargo test --workspace` | 596 passed, 0 failed, 7 ignored (was 594) |
| Production cannot use the test constructor | ✅ proven: adding `ApiState::new(..)` to library code gives `error[E0599]: no associated function named 'new'` |
| Env-read guard with `recovery.rs` removed | passes; allowlist down to three AI files plus two test-only readers |
| `cargo run --bin generate-openapi` | no diff |
| `rustfmt --check` on the four leaf files | clean |

**Mutation-tested, and it found a real gap in my own plan.**

| Mutation | First result | After fix |
| --- | --- | --- |
| Hard-code `cookie_secure: false` | ✅ caught | — |
| Hard-code the localhost Relying Party | ❌ **passed** | ✅ caught |

The test I specified asserted `webauthn_origin`, a plain `String` field, and `is_allowed_origin`, which reads
that same string. Neither looks at the `Webauthn` object, and the RP ID is what is actually baked into every
credential — the precise failure AR-006 is about. Added an assertion over
`state.webauthn.get_allowed_origins()` and re-ran the mutation to confirm it now fails. Lesson worth keeping:
assert the object the risk lives in, not a sibling field that happens to correlate.

**One deliberate behaviour change:** the API half hard-coded `language: "auto"` when building `VoiceConfig`, so
`VOICE_LANGUAGE` was honoured for a Matrix voice note and silently ignored for web dictation. Both paths now
read the setting.

### R5 — AR-007 portfolio facade (`3b49628`)

Deleted 15 forwarding methods from `PortfolioRepository`; `repository.rs` lost 293 lines. Convenience forms
(`trade::buy`, `trade::sell`, `trade::record_holding`) now sit beside the implementations they default.
`cash_repo` already had its own `deposit_cash`/`withdraw_cash`, so the facade's were exact duplicates.

**Verified:** 0 warnings; **596 tests, exactly the count before** — the check that matters, since this adds and
removes no behaviour; every remaining `PortfolioRepository::` use is one of the eight kept read/edit methods;
`openapi.json` unchanged.

The implementer caught something the plan missed: `summary()`, a *kept* method, called `Self::list_cash`
internally. Grepping for `PortfolioRepository::` would not have found it, and the crate would not have compiled.

`CLAUDE.md` named two deleted methods and still claimed no repository uses `pool.begin()`. Corrected.

### R11 — AR-009 credential resolution (`6633de7`)

**Found a real security hole while implementing, not just structure.** The extractor set an MCP token's
`last_verified_at` to the token's creation time, and `VerifiedUser` admits anything verified within five
minutes. So a freshly minted MCP personal access token opened every step-up-gated route for its first five
minutes. MCP is a machine credential with nobody to confirm anything with a passkey. It now reports the
earliest representable instant.

`accounts/principal.rs` owns what a credential is and how it resolves; the extractor is down from ~110 lines to
about 40 and keeps only the transport decision. `Credential::is_ambient()` replaces an `is_bearer` flag and
states the actual rule: only the cookie is attached by the browser unasked, so only the cookie needs a CSRF
defence.

**Verified:** 0 warnings; 597 tests; `openapi.json` unchanged; CSRF messages and `is_mutating` unchanged; all
pre-existing extractor tests pass untouched. **Mutation:** restoring `last_verified_at: mcp_token.created_at`
fails the new test.

Three distinct 401 messages collapsed into one deliberately; no test asserted on them. `SseTicketResponse`
deleted. `CLAUDE.md`'s claim that there is "no bearer token" corrected — native and MCP clients both use one.

### R2b — AR-004 AI provider settings (`ccafcce`)

`AiEnvConfig` resolves twelve environment variables once at boot; the alias chains moved verbatim, including
the order of the five endpoint aliases and seven proxy ones. `AiClient::new` takes its proxy and no longer
calls `dotenvy::dotenv()` — a library constructor reloading the `.env` file was a side effect with no business
there. `validate_ai_base_url` takes its `allow_private` flag, so what the SSRF guard permits is visible in
configuration.

The default endpoint deliberately stays at the point of use: the default *model* depends on whether the
resolved base contains `x.ai`, so folding the default in early would change which model an operator with no
`AI_MODEL` set gets.

**Verified:** 0 warnings; 597 tests; `openapi.json` unchanged; confirmed ambient `AI_ENDPOINT` and `AI_MODEL`
no longer influence resolution by running the tests with both set.

A test improved as a side effect: `test_resolved_config_defaults` guarded its assertions behind a check that
those variables were unset, so it silently skipped itself on any machine that had them.

`search.rs` keeps its reads and its allowlist entry. It sits behind the AI tool executor, whose signature chain
reaches MCP stdio mode where no `Config` is built. Tracked as **R2c**.

### R6 — AR-002 todo service (`7ffb79d`)

The big one. Four transports wrote todos and each carried its own partial copy of the rules. `modules::todo::service`
now owns validate → write → reconcile → broadcast; the transports parse and render.

**Four user-visible bugs fixed, not just structure:**

| Bug | Was |
| --- | --- |
| `!todo add` never updated an open browser | `TodoModule::handle_command` named the broadcaster `_broadcaster` and dropped it |
| A ✅ reaction left the lead-time reminder pending, firing later for a finished task | the reaction path never called the reconciler |
| AI-created todos had no length limit and never scheduled their reminder | the AI tool called the repository directly |
| Clearing completed tasks left deleted rows on screen | that path emitted no events |

The room cap now applies to every transport. Deliberate behaviour change — a limit that held on one transport
and not the others was the bug.

**Verified:** 0 warnings; 605 tests; `openapi.json` unchanged; every file under the 500-line production limit
(`api/todos.rs` fell from 414 to 278); every reaction reply string and ack emoji byte-identical, compared as
literals; **the ~30 REST handler tests pass with no assertion edited** — only three imports moved into the test
module, which is the strongest evidence the service preserved the behaviour it replaced.

**A test gap I found and closed.** `test_a_matrix_add_announces_itself` calls the handler directly with its own
broadcaster, so it passes whether or not the module threads one through — and that was the actual bug. Added
`test_the_module_passes_the_broadcaster_to_the_command` at the real `BotModule` dispatch boundary and confirmed
it fails when the regression is reintroduced. Same lesson as R4: assert at the boundary the bug lives on.

**Mutations:** disabling the length check fails both the AI-tool test and the parity table. Reverting the
reaction arm fails the reminder test.

**Interruption.** The first implementation run hit a usage limit partway through and left the tree not
compiling: the AI tool path was half-converted and the reaction path untouched. Assessed the tree, wrote a
continuation brief naming exactly what was done and what remained, and finished with a second run.

### R7 — AR-002 finance service (`72ed406`)

Same drift as todo: `FinanceModule` dropped its broadcaster so Matrix expenses never reached the browser, the
note limit existed twice under two names, the category limit was a bare literal on one path, and the AI tool
validated nothing. Also routed two writes that bypassed everything: `RecurringBillRepository::mark_paid` and
the portfolio cash bookkeeping both wrote transactions directly and announced nothing.

**A regression caught in review, and the lesson from it.** The first implementation unified category handling
by making every path create an unknown category on demand. That silently deleted a documented rule — the Matrix
path refused, with a comment explaining that implicit creation lets a typo fragment one category into two. My
brief said "use the repository's own method so there is one behaviour", which the implementer reasonably read
as licence to unify. It was not: the three transports differ on purpose and unifying them is a product decision.
Restored, with `create_missing_category` making the difference explicit at each call site, and a test that
asserts the **category row count** rather than just the error, because the side effect is the point.

Brief-writing lesson: "make it consistent" is an instruction to change behaviour unless it says which way.

### R8 — AR-002 portfolio service (`922fccd`)

Scoped down after checking. R5 had already deleted the facade and the parsing helpers were already shared, so
the remaining duplication was the parse-then-write-then-announce sequence. Two real defects: the note limit
existed only on REST, and a buy announced `PortfolioHoldingCreated` from REST and Matrix but
`PortfolioHoldingUpdated` from the AI tool. The service checks whether the symbol was already held and emits
the one that describes what happened.

Checked before acting: the frontend treats both events identically, so that inconsistency was cosmetic rather
than a live bug. Recorded rather than overstated.

**Two fixes after review.** The first version dropped the REST path's collapsing of a whitespace-only note to
`None`, so a blank field would have begun storing `''` instead of `NULL` — invisible, since both render as
nothing. Now pinned by a test and applied to every transport. And an unused `NotFound` variant kept "for parity"
with the other services was removed; nothing constructed it, and the project's own rules say not to keep
abstractions on speculation.

### R9 — AR-002 reminder service and AiError (`daf016c`, `349768f`)

Two independent changes, committed separately because they touch disjoint files.

**Reminder.** `ReminderModule` dropped its broadcaster too. The message limit was 500 over REST, 300 over
Matrix, and absent from the AI tool; the service owns one, and it is the REST value, so a Matrix reminder may
now be longer than before. A past trigger time is now rejected everywhere — equivalent to the old
"seconds must be greater than zero" on REST and Matrix, which take a duration, and a genuinely new rejection on
the AI tool, which takes an absolute timestamp.

Deliberately left alone: `due_reminder` (the shared reconciler) and the scheduler (the delivery loop) are not
transports and already announce correctly. The delete reaction keeps its own SQL because the service's cancel
only moves pending rows, and that reaction must still work on a reminder already marked triggered — a genuine
finding by the implementer, and the right call.

**AiError.** `run_ai_turn` returned `Result<_, String>` and the API mapped all of it to 400 with the internal
message in the body, so a provider outage was indistinguishable from a bad prompt. Now 400 / 503 / 500, with the
existing 504 for the outer timeout untouched.

**A test I improved during verification:** two AI tool tests carried a hard-coded 2026 trigger date that the new
past-time check had just invalidated. The implementer bumped them to 2030, which only defers the day they
expire. They now derive the date from the run.

### Known issues, not addressed

- `modules/ai/handler.rs` is 540 production lines, over the project's 500 limit. Pre-existing — it was 538
  before this series touched it.
- `modules::ai::handler::tests::test_ai_error_suppressed_from_chat_and_sent_once` flakes under full parallel
  load because of the process-global `AI_ERROR_ALERTED` flag. Passes 5/5 in isolation. Predicted in AR-011.

### R10 — AR-005 splitting the bot message handler (`44283f1`)

`register_handlers` was 487 production lines and its message closure ran for most of them, with the image and
audio branches structurally identical — mention gate, typing guard, stage indicator, resolve owner, download,
`catch_unwind` around the module call, render, link reaction targets — differing only in the module called, two
stage labels and three error messages.

| file | production lines |
| --- | --- |
| `bot/handler.rs` | 487 → **102**, pure wiring routing on `MessageType` |
| `bot/attachments.rs` | 306, the media pipeline written once |
| `bot/command_flow.rs` | 184, the text path |
| `bot/reply.rs` | 25, the strip-build-send-log sequence |

The "any text in a direct message is an AI prompt" rule moved into `CommandParser`, which owns every other rule
about what counts as a command and previously could not be asked this one. Four new tests cover it.

**Verified:** 0 warnings; 634 tests; `openapi.json` unchanged; `rustfmt` clean.

**String preservation, the check that mattered:** this file had no tests and is the bot's only Matrix entry
point, so I extracted every string literal of six or more characters from the original and diffed against the
four new files. Of 34, exactly one does not survive — a log line the shared send helper now covers with its own,
reporting the same failure with the room id. No user-facing message was lost.

The `!link`-before-resolve ordering and the comment explaining why it is load-bearing travelled with the code.
Confirmed by reading the new `command_flow.rs` directly.

**Interruption.** The implementation run hit a usage limit partway through, but unlike R6 it left the tree
compiling and complete; only the verification steps were missing. Ran those myself rather than dispatching again.

### R2c — `search.rs` environment reads: declined, with reasons

The last three entries on the environment-read allowlist are `search.rs` plus two files that only read inside
`#[ignore]`d live tests. Closing `search.rs` means threading an `AiEnvConfig` through
`ToolRegistry::execute_tool` → `executor::execute` → `execute_web_search`.

Measured the cost before starting: **35 call sites**, and the chain reaches MCP stdio mode, which deliberately
builds no `Config`. The benefit is moving six environment variables that feed one feature.

Declined as disproportionate. The project's own rules say to choose the simplest implementation that meets the
requirement and avoid indirection that does not pay for itself. The gap is already named in the allowlist with
its reason, and the guard still prevents any *new* environment read appearing anywhere. Worth revisiting only if
the tool executor is restructured for another reason, at which point the parameter comes almost free.

### R12a — AR-008 OpenAPI coverage (`e3f987a`)

Six API files carried no `#[utoipa::path]` annotations, so twenty routes were absent from `openapi.json` and
from the generated frontend client. CI diffs that document, but it cannot guard an endpoint that was never in
it: a field could be renamed on any of those routes and nothing would notice.

**Verified here, independently of the implementation report:**

| Check | Result |
| --- | --- |
| `cargo check --workspace --all-targets` | 0 warnings |
| `cargo test --workspace` | 635 passed, 0 failed, 7 ignored |
| `openapi.json` | 87 → 107 paths, regenerates byte-identically |
| `frontend/src/types/api.ts` | regenerates identically; `npm run build` succeeds |
| Route guard mutation | removing a handler from `paths(...)` fails the test naming `/api/v1/footprints/stats` |

The guard **parses `routes.rs` at test time** rather than keeping a second list of routes beside them. That is
now the fourth guard in this shape, after the ownership tables, the no-`crate::api` walk and the environment-read
walk. It converts Axum's `:id` to OpenAPI's `{id}`, follows the `nest` into `mcp.rs`, and allowlists five
endpoints with no schema to describe.

**Two defects surfaced.** Two further routes were undocumented beyond the six files. And `habits` and `bookmarks`
both defaulted to the operationId `get_stats`, producing a duplicate identifier in the generated TypeScript;
`habits` now sets an explicit one, following the `portfolio_summary` precedent.

### Process note — two sessions, one working tree

Partway through R12a a second Claude session was found working the same roadmap against the same branch and
working tree. It had been given a brief asking for a hand-maintained `ROUTE_PATHS` constant — the exact
"hand-maintained mirror" anti-pattern this review named as root cause RC-D — and re-added it twice while this
session was removing it, breaking the build both times.

Resolved by message: the derived version stands, the peer stopped, and `ROUTE_PATHS` is absent from the tree.

Worth recording as a risk rather than an anecdote. The two sessions came close to committing contradictory
versions of the same test, and nothing in the tooling would have caught it — both were running with the same
permissions against the same checkout. Surfaced to the user, because whether two sessions should be running at
once is their call.

### Formatting — the workspace is rustfmt-clean (`a76fb8f`)

F-3 recorded that `cargo fmt --all -- --check` was failing at the branch point, 54 diffs across 18 files, and
that CI runs exactly that command. Every item in this series formatted only the files it touched, deliberately,
because running the formatter over the whole workspace would have buried the diff of whatever was being
changed. With the series committed that reason is gone, so it is now done as its own commit.

Formatting only. Three files differ at token level because rustfmt adds braces around single-expression closure
and match-arm bodies (`Ok(false) => Some(x)` becomes `Ok(false) => { Some(x) }`); checked each rather than
assumed. 635 tests, 0 warnings, `openapi.json` regenerates unchanged, and **CI's format step passes for the
first time on this branch**.

This also retires a workaround: `modules/todo/mod.rs` had been hand-formatted during the series because rustfmt
recurses into the submodules a `mod.rs` declares and would have reformatted its siblings. Not needed now.

## Where this stands

Everything on the roadmap is committed and verified except **R12b**, which the second session has taken
(generated DTOs replacing the hand-written module types, and a registry-driven Sidebar), and **R2c**, declined
with reasons above. Two items the second session deferred rather than dropped, and which are not done:

- Replacing the `window` custom-event navigation with a small store. Churn for modest gain.
- Folding `modules/journal` into `modules/life`. Backend, and it changes command routing, so it wants its own
  item rather than riding along in a frontend change.

Known issues still open: `modules/ai/handler.rs` is over the 500-line production limit (pre-existing), and
`test_ai_error_suppressed_from_chat_and_sent_once` flakes under full parallel load because of a process-global
flag (predicted in AR-011).

### R12b — AR-008 generated DTOs (`f938e30`)

**This item found a live production bug, which is the whole argument for doing it.**

`frontend/src/modules/finance/components/NetWorthStatStrip.tsx` read `data.native_breakdowns`,
`data.fx_source` and `data.portfolio_cash_minor`. The server returns `currencies`, `fx_rates` and
`cash_deposits_minor`, and has since the widget shipped in `dff0c4c`. Line 35 did
`data.native_breakdowns.length`, so the component threw on render for anyone whose Finance or Portfolio page
loaded it.

Verified against the backend rather than taken on report: `net_worth.rs::NetWorthResponse` has no serde
renames, and the generated client agrees. The three fields simply do not exist.

**Why no test caught it.** `frontend/e2e/fake-api.ts` mocked the shape the *frontend type* claimed, not the one
the server returns. The tests were asserting against a response the API has never produced. Fixing the
component made the mock's fiction visible as a crash across `finance`, `portfolio` and `navigation` specs. The
mock now mirrors `net_worth.rs` and carries a comment saying so.

That is worth generalising: a hand-written client type plus a mock written to match it will agree with each
other forever and with the server never.

**Other mismatches found:** `LifeHorizonDay` claimed `index`, `year` and `status`, none of which the server
sends — they are computed client-side, so the wire type and the calendar view model are now separate.
`LifeProfile.id` and a nested `stats` object the server never returned are gone, with the dead fallback chain
that read them.

About 65 interfaces became aliases. A dozen stay hand-written with a stated reason: filter unions, form drafts,
a Nominatim result, view models assembled from several responses.

**Verified:** `npm run build` (runs `tsc`) passes; Playwright 184/185 and 183/185 across two runs with the
failures moving between passkey specs I never touched — pre-existing load flakiness, confirmed by the failing
test passing 3/3 in isolation.

### Backend findings from R12b, not fixed here

`TodoPriority`, `RecurrencePattern` and `ReminderStatus` generate enum schemas from Rust variant names, so they
advertise capitalised values (`"Medium"`) where the wire carries lowercase (`"medium"`). Adopting them would
have introduced wrong values, so those fields stay `string`. The schemas should be corrected or dropped.

`frontend/src/modules/dashboard/index.tsx` still has local duplicate DTOs, outside this item's file list.

### Sidebar — not done, deliberately

`Sidebar.tsx` carries information the registry cannot express: labels that differ from module names on purpose
("Tasks" vs "Tasks & Todos"), an icon for `recurring_bills` that contradicts the module's own `iconName`, and
five modules declaring no icon at all. Driving it from the registry would have changed rendered output or
required duplicating fields. Skipped rather than half-done.
