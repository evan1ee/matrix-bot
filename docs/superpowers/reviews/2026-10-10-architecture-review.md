# Architecture review: `matrix-bot` backend

**Date:** 2026-10-10
**Status:** documentation findings fixed (2026-10-11); ADR-A and ADR-B are still Proposed
**Branch reviewed:** `feat/config-management` (working tree, including uncommitted changes)
**Scope:** `backend/src/` layering, boundaries, runtime ownership and documentation fidelity, measured against [`AGENTS.md`](../../../AGENTS.md) and [`docs/architecture.md`](../../architecture.md).
**Out of scope:** the configuration and secrets vault feature itself, which already has a line-by-line review in [`2026-10-10-config-management-review.md`](2026-10-10-config-management-review.md). This review refers to that one instead of repeating it.

## 1. Verdict

The architecture holds up. It is a modular monolith with clear layers, about 40 mechanical architecture tests, one transactional outbox, one durable job queue and one channel abstraction that has four real implementations. The big decisions (PostgreSQL only, outbox instead of an in-process bus, jobs instead of sync-loop timers, `Arc<dyn ChannelAdapter>` as the only trait-object seam) are sound, recorded and enforced. Nothing here calls for structural change.

There are two kinds of gap. Both come from the guard rails covering less than the rules they stand for:

1. **The LLM and AI paths go around the service layer.** `llm/` and parts of `interface/` call repositories and the pool directly, even though `LlmService` exists. The architecture test does not check `llm/`, and its "no SQL in interface" rule passes repository calls.
2. **Some rules are stated but enforced only in places.** The 500-line cap is checked for `agent` and `knowledge` only. "One env name per setting" and "never a detached task" have no check. "No external calls from tests" is broken in at least one test.

Fixing both is mostly moving code, not redesigning anything.

| Area | Rating | Notes |
|---|---|---|
| Layering and dependency direction | Good | Enforced by `backend/tests/architecture.rs`; exceptions are in §3 |
| Service layer as the single capability path | **Partial** | LLM conversations and config, net worth, retrospective (H1, H2) |
| Transactions and the outbox | Good | `publish` takes `&mut Transaction`; only `application/` publishes |
| Jobs, events and shutdown | Good | One `CancellationToken` tree with a 15 s bound; a few detached tasks in the Matrix adapter (M3) |
| Channel abstraction | Good | Four adapters; each SDK is confined to its own directory by tests |
| Abstraction discipline | Good | 11 traits, each with a real second implementation or a test seam; one dead compatibility shim (L2) |
| File and function size | Mostly good | One production file over 500 lines; 83 files between 300 and 500 lines (M2) |
| External-service hygiene | **Partial** | FX feed reached from a read model and from a test (H2); env read at call time (M1) |
| Documentation fidelity | **Partial** | Some statements are stale or wrong (§5) |

## 2. What is working well

These should be kept and not reopened:

- **Mechanical enforcement.** `tests/architecture.rs` has about 40 rules: domain purity, no transport crates in `application/`, no SQL in `interface/`, `jobs/` or `events/`, each SDK confined to its adapter, and only `application/` publishing to the outbox. Most rules in AGENTS.md cannot be broken without a red test.
- **Outbox by construction.** `repositories::outbox::publish(&mut tx, …)` cannot be called outside a transaction, so "event without state change" cannot be written. The listener's replay window and the dispatcher's lease fencing (fenced by `attempts`, retry time on the database clock) are correct designs for at-least-once delivery.
- **Ownership is carried, not derived.** `AccountId` newtype, `AuthUser::owner_id()`, the `OWNED_TABLES` merge list and `UNIQUE (platform, platform_user_id)` make tenant isolation structural.
- **Restraint about abstraction.** Repositories are free functions over `impl PgExecutor`, with no repository traits or provider enums. Each trait that exists has two real implementations (`ChannelAdapter`, `StorageService`, `EmbeddingProvider`, `SecretProtector`) or is a narrow test seam (`Outbound`, `DeliverySink`, `ProgressSink`, `JobHandler`).
- **Bounded shutdown.** One token tree, a 15 s deadline, `abort` instead of detach for stragglers, and `shutdown_background()` for the blocking pool.
- **Degrade one feature, not the process.** OCR, STT, TTS and embeddings each turn off on their own when their endpoint is unset or down.

## 3. Findings

Severity follows the user's code-review scale: HIGH means a bug or significant quality issue that should be fixed before the next feature lands; MEDIUM means a maintainability concern; LOW is optional.

### H1 — The LLM conversation path bypasses `LlmService` (HIGH)

**Evidence**

| Caller | Direct calls |
|---|---|
| `llm/turn.rs:9,40,89,94,106,115,136,247,300` | `llm_repo::get_user_config`, `get_conversation`, `get_or_create_conversation`, `get_recent_messages`, `add_message` |
| `interface/http/ai.rs:8,108,128,133,183,299` | `llm_repo::list_conversations`, `get_conversation`, `get_recent_messages`, `get_user_config` over `state.db.pool` (7 pool uses) |
| `interface/chat/llm.rs:11,298` | `llm_repo::get_user_config` |

`application/llm/mod.rs` already defines `LlmService` with `get_user_config` and a transactional `update_user_config` that publishes `AiConfigUpdated`. It is in `AppServices`, but these three callers skip it.

**Why it matters**

- This breaks rules 2 and 3 of AGENTS §2. Three interfaces get AI config and conversation history on their own, so a rule added to `LlmService` (redaction, retention, per-account limits, the config precedence B3/B4 from the config review) will not apply on the paths that matter most.
- `turn.rs:247,300` discard `add_message` errors with `let _ =`. A conversation can silently lose a turn, which goes against the "never swallow an error" rule.
- `interface/http/ai.rs` is the only HTTP module that still reads the pool for business data. `ApiState`'s own comment says handlers "call these instead of reaching repositories or the pool".

**Why the tests miss it:** `interface_runs_no_sql` and `no_sqlx_in_interface_or_application` look for SQL tokens and `sqlx`, not for `infrastructure::postgres` imports. `llm/` is not checked at all; AGENTS says "keep `llm/` to it too", but that is not enforced.

**Fix:** see ADR-A in §6. Move conversation reads and writes into `LlmService` (`conversation_for_turn`, `append_message`, `history`, `list_conversations`). Make `llm/turn.rs` take `&AppServices`. Add an architecture rule saying `interface/` and `llm/` may not import `infrastructure::postgres` outside `#[cfg(test)]`.

### H2 — Net worth is a business calculation inside an infrastructure read model that calls the network (HIGH)

**Evidence**

- `infrastructure/postgres/read_models/net_worth.rs:14,26` imports `infrastructure::web::fx::get_rates` and fetches ECB rates over HTTP from inside a "postgres read model".
- It is called directly from two interfaces with no service between them: `interface/http/finance_net_worth.rs:93` (HTTP handler) and `llm/tools/portfolio/net_worth.rs:6,21` (AI tool, which also builds its own `reqwest::Client`).
- `net_worth.rs:234-235` (test) builds a real `reqwest::Client` and calls `compute_net_worth`. On a cold cache this contacts `www.ecb.europa.eu`. That breaks AGENTS §9 ("never contact … external service from tests") and makes the test flaky without network access.
- `infrastructure/web/fx.rs:23` keeps the cache in a process-global `static FX_CACHE: RwLock<…>`. AGENTS §3.3 says to avoid global mutable state, and the global also leaks state between tests.

**Why it matters:** currency conversion and aggregation across ledger, cash and holdings are business rules. Here they live in an infrastructure module that mixes SQL with outbound HTTP, so no service owns "net worth". Two interfaces reach infrastructure directly, and the test suite depends on the network.

**Fix:** see ADR-B in §6. Keep the SQL aggregation as a read model, move the FX conversion into a pure `domain::finance` function (rates in, totals out), and add a `FinanceService::net_worth(owner)` (or `PortfolioService`) that fetches rows and rates and calls the domain function. Inject the FX client into the service so tests can use fixed rates. Both interfaces then call the service.

### H3 — `llm/retrospective.rs` reads six repositories directly (HIGH)

**Evidence:** `llm/retrospective.rs:10-15` imports `finance`, `habits`, `life`, `recurring_bills`, `reminders` and `todos` repositories and queries them at lines 91, 206, 211, 230, 280 and 294.

**Why it matters:** AGENTS §5.3 says LLM executors "call services, never repositories". Retrospective builds a cross-feature summary that reads around six services, so any visibility rule a service applies (soft deletes, archived items, per-feature limits) is skipped. It is also a second, unchecked aggregation next to `BriefingService`, which does a similar cross-feature read.

**Fix:** move the gathering into an application use case (extend `BriefingService` or add `application/retrospective`) that calls the feature services. `llm/` keeps only prompt assembly. The architecture rule from H1 then covers this file too.

### M1 — Configuration read from the environment at call time, outside `config/` (MEDIUM)

**Evidence:** `llm/search.rs:93` reads `TAVILY_API_KEY`. `llm/search.rs:153-157` reads `AI_PROXY`, `HTTPS_PROXY`, `https_proxy`, `ALL_PROXY` and `all_proxy` on every search call, building a new client each time.

**Why it matters:** AGENTS §5.3 says "Settings read exactly one env name each", and the config work just added a precedence order (snapshot > env > default) that these reads skip. A five-name proxy fallback makes the effective setting hard to diagnose, and env reads at call time cannot be validated at boot by `Config::validate`.

**Fix:** read each value once in `config/` (one name each, documented in `.env.example`), keep it in `Config` or the runtime snapshot, and pass a pre-built `reqwest::Client` with the proxy set. Delete `get_tavily_api_key` once the snapshot path is the only one.

### M2 — The 500-line production cap is enforced for two features only (MEDIUM)

**Evidence:** `tests/architecture.rs:1006` (`agent_files_do_not_exceed_500_production_lines`) and `:1098` (`knowledge_…`) check only those feature directories. A scan of production lines (up to the first `#[cfg(test)]`) finds `application/secrets/mod.rs` at 514 lines, plus `domain/config/registry.rs` at 568 according to the config review. 83 more files are between 300 and 500 lines, above the 300-line target.

**Fix:** replace the two per-feature tests with one test over `backend/src/` and `inference/src/`. It should skip whole-file test modules (`*tests.rs`, `test_support.rs`, `testing.rs`, `fake.rs`) and allow-list current violations by name, so the list can only shrink. Split `application/secrets/mod.rs` along its seams (CRUD and audit vs connection testing).

### M3 — Detached tasks in the Matrix adapter (MEDIUM)

**Evidence:** `tokio::spawn` with the handle dropped and no cancellation token:

| Site | Task |
|---|---|
| `interface/channels/matrix/client.rs:358` | `delete_stale_device` (UIA network call) |
| `interface/channels/matrix/client.rs:389` | recovery and self-verification (sleeps, then retries) |
| `interface/channels/matrix/verification.rs:374` | `drive_request` (SAS flow, can wait on a human) |
| `application/accounts/principal.rs:38` | MCP token `touch_last_used` (bounded by pool acquire timeout; documented) |

**Why it matters:** AGENTS §4.3 says anything long-lived gets a child `CancellationToken` and joins `app::run`'s shutdown, "never a detached task". The self-verification task is the critical one: it can still be running when a reconnect makes a new client generation, so two generations may sign at once. On shutdown it is killed by `shutdown_background()` instead of being cancelled. `stage.rs:26` keeps its handle and is fine.

**Fix:** pass the generation's child token into these spawns and `select!` against it. Keep the verification and recovery handles in the generation so a reconnect cancels them. The `principal.rs` touch can stay, but its comment should say why it is exempt.

### M4 — Architecture tests check tokens, not dependency direction (MEDIUM)

H1–H3 were only possible because the test looks for SQL text and crate names but not for the layer path `crate::infrastructure::postgres`. Add two rules:

- `interface/` (production) may not import `infrastructure::postgres`. The `read_models` module is allowed only through an application service.
- `llm/` (production) may not import `infrastructure::postgres` or run SQL, which makes AGENTS' "keep `llm/` to it too" enforceable.

Both rules need allow-lists until H1–H3 land, and the lists should be emptied by those changes.

### L1 — `AppServices` is wide, and `ApiState` exposes the pool next to it (LOW)

`AppServices` has 26 public fields, and `ApiState` carries both `services` and `db`. The width matches the feature count and is fine for a composition root. But having `state.db.pool` in every handler is what makes H1 easy to write. Once H1 lands, the only production users of `state.db` in `interface/http` are `health.rs` (ping) and `mcp.rs`. Consider replacing `db` with a narrow `HealthProbe` so the pool is no longer within reach of handlers.

### L2 — A dead compatibility shim (LOW)

`llm/search.rs:85-89` `grok_search_tools()` is labelled "Backward compatibility" and has no callers. AGENTS §4.3 says "Deletion over compatibility". Delete it.

### L3 — FX rates as `f64` (LOW, check)

`domain::finance::fx` keeps rates as `HashMap<String, f64>`. Amounts stay in integer minor units, which is correct, but converting through a float rate can still be off by one minor unit on large sums. When H2 moves conversion into the domain, pin the rounding rule (half-even, applied once per converted total) and test it.

## 4. Risk map

```text
                 likelihood of drift
                 low                      high
           ┌──────────────────────┬──────────────────────┐
 impact    │ M3 detached tasks    │ H1 LLM bypasses svc  │
 high      │                      │ H2 net worth in infra│
           ├──────────────────────┼──────────────────────┤
 impact    │ L1 ApiState.db       │ M2 size cap partial  │
 low       │ L2 dead shim, L3 FX  │ M1 env reads, M4     │
           └──────────────────────┴──────────────────────┘
```

The top-right quadrant has the same cause for every item: a rule with no mechanical check. M4 is the cheapest change that stops these from coming back.

## 5. Documentation drift in `docs/architecture.md`

| Line | Says | Actually |
|---|---|---|
| 47 | Infrastructure is `infrastructure/{postgres,inference,web}/` | `crypto/` and `storage/` exist too |
| 113 | "The webhook delivery queue is unchanged until step 9 (outbox)" | Step 9 has landed; webhooks go through the outbox (lines 86–109) |
| 145 | "Matrix and Discord now, then Telegram and Email" | All four adapters are implemented (lines 150–151) |
| 297 | `backend/src/error.rs::ApiError` | `backend/src/interface/http/error.rs` (the `error_module_is_in_interface_http` test enforces this) |
| 308 | HNSW "guaranteeing sub-millisecond … retrieval under concurrent load" | No measurement backs this; state the index parameters only |
| 320–349 | "Dynamic Configuration" section | Facts match the working tree (admin CLI, boot-time snapshot load, SSRF-guarded testers), but headings were Title Case and one line described a test-key bypass that has since been removed |

**Status (2026-10-11):** all six fixed in `docs/architecture.md`.

## 6. Proposed decisions

### ADR-A: AI conversation state is owned by `LlmService`

**Status:** Proposed
**Date:** 2026-10-10
**Deciders:** repository owner

**Context.** Conversation history and per-user AI config are read and written from `llm/turn.rs`, `interface/http/ai.rs` and `interface/chat/llm.rs` directly through `repositories::llm`. `LlmService` exists but covers only config get and update. Config precedence (user > snapshot > env > default) is being added in the config-management work, and a precedence rule implemented in three places will drift.

**Decision.** `LlmService` is the only caller of `repositories::llm`. It exposes `conversation_for_turn(owner, conversation_hint)`, `append_message(owner, conversation, message)` (returns `Result`, never ignored), `history(owner, conversation, limit)`, `list_conversations(owner, limit)` and `effective_config(owner)`, which resolves precedence once. `llm/turn.rs` receives `&AppServices`.

**Options considered**

| | A1: Route through `LlmService` (chosen) | A2: Leave as is, document the exception | A3: New `ConversationService` |
|---|---|---|---|
| Complexity | Low: move about 10 calls | None | Medium: second service over the same tables |
| Consistency with AGENTS | Full | Breaks rules 2 and 3 | Full |
| Risk | Low; behaviour unchanged | Precedence and redaction drift across 3 paths | Splits one feature's ownership |

**Consequences.** Easier: a single place for retention, redaction and config precedence; one architecture rule can forbid the bypass. Harder: `llm/turn.rs` tests need a service rather than a pool (they already have `test_pool()`). Revisit if conversation storage ever moves off PostgreSQL.

**Action items**
1. [ ] Add the service methods with transaction and error tests.
2. [ ] Switch the three callers over and remove the `let _ =` on `add_message`.
3. [ ] Add the M4 architecture rule for `llm/` and `interface/`.

### ADR-B: Net worth is an application use case over a pure domain calculation

**Status:** Proposed
**Date:** 2026-10-10
**Deciders:** repository owner

**Context.** `compute_net_worth` sits in `infrastructure/postgres/read_models` but does outbound HTTP (ECB), currency conversion and aggregation. Two interfaces call it directly, and its test reaches the network.

**Decision.** Split it into three parts: (1) `read_models::net_worth::positions(owner)`, SQL only, returning ledger, cash and holdings rows in minor units; (2) `domain::finance::net_worth(positions, rates, primary_currency)`, pure, with a pinned rounding rule; (3) `FinanceService::net_worth(owner)`, which gets positions, gets rates from an injected FX client (timeout, cache owned by the service rather than a `static`) and calls the domain function. The HTTP handler and AI tool call the service.

**Options considered**

| | B1: Service + pure domain (chosen) | B2: Move the whole function into `application/` | B3: Keep in infrastructure, inject the client |
|---|---|---|---|
| Complexity | Medium | Low | Low |
| Testability | Domain is unit-testable with fixed rates; no network | Still mixes SQL and HTTP in one function | Test seam only |
| Layering | Correct | Acceptable | Business rule stays in infrastructure |

**Consequences.** Easier: deterministic tests, one owner for net worth, a cache with a lifetime. Harder: one more service method and a small FX client handle in `AppServices`. Revisit if a second FX source gets added (the client may then need a trait; for now it does not, per AGENTS §3.2).

**Action items**
1. [ ] Pure domain function with rounding tests.
2. [ ] Service method plus an injected FX client; drop `static FX_CACHE`.
3. [ ] Switch both callers; delete the network-touching test.

## 7. Recommended order

1. **Finish the config-management blockers first** (B1–B8 in the config review). They are security and correctness issues on the branch about to merge, and ADR-A depends on B3/B4's precedence resolution.
2. **M4 architecture rules with allow-lists.** This is cheap and stops new bypasses while the rest is done.
3. **ADR-A (H1)**, then **H3**, emptying the allow-list as each lands.
4. **ADR-B (H2)**, which also removes the network test.
5. **M1, M2, M3** as housekeeping, one commit each.
6. **§5 doc fixes**, in a single `docs:` commit or alongside each change.

Record ADR-A and ADR-B as rulings in `docs/superpowers/HANDOFF.md` when they are accepted (`Ruling: <decision> — <why> — <cost if wrong>`).

## 8. Method

These checks were run against the working tree on 2026-10-10. Read-only; nothing was built or executed.

- Read `AGENTS.md`, `docs/architecture.md`, `docs/superpowers/HANDOFF.md` and the config-management review.
- Lines and files per layer: `interface` 54k lines over 192 files, `infrastructure` 22k, `application` 18k, `domain` 13k, `llm` 9.5k.
- Production line count per file (lines before the first top-level `#[cfg(test)]`) across `backend/src` and `inference/src`.
- `grep` for `infrastructure::postgres`, `repositories::`, `sqlx::`, `state.db.pool`, `tokio::spawn`, `std::env::var`, `unwrap()`/`expect(` and `pub trait`, with each hit classified as production or test by position relative to the `#[cfg(test)]` boundary.
- Read each cited call site to confirm the finding.

This review did not run the §8 gates (`cargo test`, `clippy`, etc.); the config review records the latest gate results. Findings H1–H3 and M1–M3 are confirmed by reading the code. L3 is a risk to check, not an observed defect.
