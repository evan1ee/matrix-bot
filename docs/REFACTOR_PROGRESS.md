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
| R2 | AR-004 — `Config` threaded into the bot side | — | — | — | Next up |
| R4 | AR-006 — `BotHandles`, `ApiState::from_config` | — | — | — | |
| R5 | AR-007 — collapse the portfolio facade | — | — | — | |
| R11 | AR-009 — credential resolution extracted | — | — | — | |
| R6–R9 | AR-002 — per-module service layer | — | — | — | |
| R10 | AR-005 — split `bot/handler.rs` | — | — | — | |
| R12 | AR-008 — OpenAPI coverage, generated DTOs, registry-driven nav | — | — | — | |

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
