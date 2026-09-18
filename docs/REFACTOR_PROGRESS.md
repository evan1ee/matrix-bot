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
| R1a | AR-001 — ownership-table drift, classification, transactional merge | ✅ | — | — | In progress |
| R1b | AR-001 — `portfolio_audit_log` merge semantics | — | — | — | Split out of R1; see finding below |
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
