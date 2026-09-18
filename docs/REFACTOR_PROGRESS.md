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
| R1b | AR-001 — `portfolio_audit_log` merge semantics | — | — | — | Split out of R1; see F-1. Next up |
| R3 | AR-003 — move misplaced `api/` functions into `modules/`+`db/` | — | — | — | |
| R2 | AR-004 — `Config` threaded into the bot side | — | — | — | |
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
