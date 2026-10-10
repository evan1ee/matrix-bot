# Technical debt audit — 2026-10-10

**Scope:** whole repository (`backend/`, `inference/`, `frontend/`, CI, docs) at `feat/config-management` (`ade0288` plus the uncommitted working tree: 66 files changed, apparently remediating the config-management review).
**Companion:** [`2026-10-10-config-management-review.md`](2026-10-10-config-management-review.md) holds the branch-specific findings (B1–B8, M1–M18). This audit does not repeat them; it tracks them as one item (TD-02).

## Method

Evidence came from the code, not from memory: production line counts (lines before the first top-level `#[cfg(test)]`), `#[allow]`/`TODO` greps, `cargo tree -d`, `Cargo.lock`, the CI workflow, the plan/spec folder, and the open items in [`HANDOFF.md`](../HANDOFF.md) §4 Track 2.

**Scoring** (each 1–5): Impact = how much it slows work; Risk = cost of leaving it; Effort = size of the fix (1 = under an hour, 5 = multi-week).
**Priority = (Impact + Risk) × (6 − Effort).** Maximum 50.

## Prioritized register

| ID | Item | Category | I | R | E | Priority | Phase | Status |
|---|---|---|---|---|---|---|---|---|
| TD-01 | Scheduled agent tasks ignore their tool whitelist | Code / security | 3 | 5 | 2 | **32** | 0 | **Done** |
| TD-03 | Playwright e2e is not run in CI | Test / infra | 4 | 4 | 2 | **32** | 1 | **Done** |
| TD-02 | Config-management review findings not yet committed | Code / security | 4 | 5 | 3 | **27** | 0 | **Done** |
| TD-04 | Completed plans left in `docs/superpowers/plans/` | Documentation | 3 | 2 | 1 | **25** | 1 | **Done** |
| TD-05 | Hand-written e2e fakes and frontend types drift from OpenAPI | Test | 3 | 4 | 3 | **21** | 1 | Open |
| TD-06 | Doc drift in AGENTS.md and the reference docs | Documentation | 2 | 2 | 1 | **20** | 1 | **Done** |
| TD-07 | `domain/` imports `sha2` against the layer rule | Architecture | 2 | 2 | 2 | **16** | 1 | **Done** |
| TD-08 | `application/secrets/mod.rs` over the 500-line cap; many files close to it | Code | 2 | 2 | 2 | **16** | 1 | Open |
| TD-09 | `#[allow]`s with no reason, and dead code | Code | 2 | 1 | 1 | **15** | 1 | **Done** |
| TD-10 | Three `reqwest` / two `hyper` stacks; direct `reqwest 0.11` | Dependency | 2 | 3 | 3 | **15** | 2 | Open |
| TD-11 | No integration test for the portfolio price-refresh snapshot | Test | 1 | 2 | 1 | **15** | 1 | Open |
| TD-12 | Frontend has no unit tests; large components | Test / code | 2 | 2 | 3 | **12** | 2 | Open |
| TD-13 | Deferred edge cases: `SyncHealth` parse, briefing window wrap-around | Code | 1 | 2 | 2 | **12** | 2 | Open |
| TD-14 | 23 `too_many_arguments` sites | Code | 2 | 1 | 3 | **9** | 2 | Open |
| TD-15 | Any quoted message taints the AI turn, so quote-based confirmation fails | Code / UX | 2 | 1 | 3 | **9** | 3 | Open |
| TD-16 | Major-version lag: axum 0.7, utoipa 4, tower-http 0.5, thiserror 1 | Dependency | 2 | 2 | 4 | **8** | 3 | Open |
| TD-17 | Built-in agent tasks are stubs marked `[TODO Extension]` | Code | 2 | 2 | 4 | **8** | 3 | Open |

---

## Item detail

### TD-01 — Scheduled agent tasks ignore their tool whitelist (32)
**Evidence:** `ScheduledAgentTask::allowed_tools()` returns a list for each built-in task (`domain/agent/task.rs`). But both job entry points pass `allowed_tools: None`: `interface/jobs/agent/mod.rs:159` and `interface/jobs/agent/resume.rs:131`. As a result, a background run can call any registered tool. That includes mutating tools that its definition was meant to exclude.
**Why it matters:** The agent runs without supervision, on prompts that include untrusted content such as bookmarks and notes. A whitelist that is declared but not enforced gives a false sense of safety. AGENTS §5.3 requires authorization that does not depend on the model.
**Fix:** Pass the task's whitelist through the job payload or context into `evaluate_step` and `execute_approved`. Add a test showing that a tool outside the list is refused, including on the resume path. **Effort:** about half a day.

### TD-02 — Config-management review findings not yet committed (27)
**Evidence:** The review lists 8 HIGH blockers and 18 MEDIUM findings. The working tree contains a large uncommitted diff, and spot checks show some fixes are in it: B2 (`without_url()` in `testers/mod.rs`), B5 (the listener no longer cancels `listen` from a TTL branch), M8 (the `test`/`mock` key bypass is gone), M9 (no real-network test URLs remain), and M12 (`registry.rs` is now 454 production lines).
**Why it matters:** Until these fixes are committed, verified with the §8 gates, and checked off in the review doc, merging `feat/config-management` carries a secret-leak risk and a crash in the Admin console.
**Fix:** Work through the review's §6 order. Mark each finding fixed or deferred in the review doc. Run the full gate set and commit in logical pieces. **Effort:** depends on what remains; the diff suggests most of it is done.

### TD-03 — Playwright e2e is not run in CI (32)
**Evidence:** The `frontend` job in `.github/workflows/ci.yml` runs only `npm ci` and `npm run build`. The 222 Playwright tests (in `frontend/e2e/`) run only when someone remembers to run them locally. AGENTS §8 calls them "required before shipping a UI change".
**Why it matters:** UI regressions can merge without anyone noticing. Because the specs use a fake API, running them in CI costs little: no backend and no database are needed.
**Fix:** Add a `npx playwright install --with-deps chromium && npm --prefix frontend run test:e2e` step and upload the trace when it fails. **Effort:** 1–2 hours.

### TD-04 — Completed plans left in `docs/superpowers/plans/` (25)
**Evidence:** There are 28 plan files, and 21 of them contain unchecked `- [ ]` boxes, about 730 in total, for work that HANDOFF reports as finished (inference, outbox, job queue, omnichannel, notes refactor and others). One example: `2026-10-04-technical-debt-remediation.md` still shows Task 1 (R165) open, but `application/identity/link.rs:397` implements it.
**Why it matters:** AGENTS §10 says "completed history is deleted rather than accumulated". A new agent that reads these plans can easily redo finished work or trust a stale design.
**Fix:** Delete the plans and specs whose work is finished and shipped. Keep the design specs that are still the authority for live behavior, or fold the parts that still matter into `docs/architecture.md`. **Effort:** under an hour.

### TD-05 — Hand-written e2e fakes and frontend types drift from OpenAPI (21)
**Evidence:** Review finding B6: the audit endpoint returns `AuditLogPageDto`, but both the fake and the hook assumed a bare array. Every spec passed while the real UI crashed. `frontend/src/modules/admin/types.ts` duplicates DTOs that `frontend/src/types/api.ts` already generates (M18). `frontend/e2e/fake-api.ts` builds responses by hand.
**Why it matters:** The fakes are the only thing the e2e tier tests against. When they drift from the real API, green tests prove nothing.
**Fix:** Make the modules import their types from the generated `api.ts`. Type the fake-api handlers against the same types, so a shape mismatch fails `tsc`. Add a CI check that `npm run generate-api` produces no diff. **Effort:** 1–2 days across modules; do it module by module.

### TD-06 — Doc drift (20)
**Evidence:**
- `AGENTS.md:236` says clippy is "not yet a CI job". But `ci.yml:112` and `:235` both run clippy with `-D warnings`.
- The AGENTS §8 clippy command uses `-p matrix-bot`, while the HANDOFF §6 command uses `--workspace`.
- Review M11 lists reference docs (`database.md`, `architecture.md`, `api.md`) that describe columns and types that do not exist.

**Fix:** Correct the lines, and make HANDOFF §6 point to AGENTS §8 instead of keeping a second copy of the command list. **Effort:** under an hour, plus M11.

### TD-07 — `domain/` imports `sha2` against the layer rule (16)
**Evidence:** `domain/accounts/session.rs:23`, `domain/portfolio/audit.rs:5` and `domain/webhooks/mod.rs:7` all `use sha2`. AGENTS §3.1 limits `domain` to `std`, `chrono`, `uuid`, `serde` and `thiserror`. `tests/architecture.rs` forbids `webauthn_rs` but not `sha2`, so the violation passes CI.
**Fix:** Move hashing to `utils::secure` or the application layer, and pass the domain the digests it needs. Then add `"sha2"` to the forbidden list. **Effort:** half a day.

### TD-08 — File size cap (16)
**Evidence:** `application/secrets/mod.rs` has **514** production lines, over the 500 cap. Close behind: `interface/http/life.rs` 499, `repositories/reminders.rs` 494, `http/auth/types.rs` 490, `repositories/portfolio/trades.rs` 488, `matrix/attachments.rs` 482, `domain/accounts/mod.rs` 478, `identity/link.rs` 473, `chat/knowledge.rs` 471. About 40 files exceed 400 lines, against a target of under 300.
**Fix:** Split `secrets/mod.rs` now, along a real seam such as CRUD versus connection testing and auditing. For the others, split each one the next time a feature touches it. A sweep-only refactor would break the rule against mixing features and refactors. **Effort:** 2 hours for secrets.

### TD-09 — `#[allow]`s with no reason, and dead code (15)
**Evidence:** AGENTS §9 requires every `#[allow]` to document why. None of these do:
- `#[allow(dead_code)]` on four functions in `interface/http/reminders.rs:24-67` (`as_str`, `from_str_or_default`). These are probably truly dead; delete them.
- `#[allow(dead_code)]` at `http/auth/matrix.rs:136` and `chat/mod.rs:192`.
- `#[allow(unused_imports)]` on `ApiErrorResponse` in `http/agent.rs`, `http/storage.rs` and `http/auth/matrix.rs`, plus `inference/src/{ocr,stt,tts}/mod.rs`. The import is probably needed only inside `utoipa` attributes; if so, reference it by path and drop the allow.

**Fix:** Delete the dead code and drop the allows, or add a one-line justification. **Effort:** 1 hour.

### TD-10 — Duplicate HTTP stacks (15)
**Evidence:** `Cargo.lock` has `reqwest` 0.11.27, 0.12.28 and 0.13.5, `hyper` 0.14 and 1.x, `http` 0.2 and 1.x, and `h2` 0.3 and 0.4. The backend's direct dependency is `reqwest 0.11`, which is on the old hyper-0.14 line. `base64` exists in three versions.
**Why it matters:** Longer compile times, a larger binary, and patch-only maintenance on the 0.11 line. When a security advisory lands on one stack, the others don't get its fix.
**Fix:** Move the backend and inference crates to `reqwest 0.12`; the API is close to identical, and `without_url()` and the timeouts carry over. Then re-run `cargo tree -d`. **Effort:** half a day to a day, mostly test reruns.

### TD-11 — Portfolio price-refresh snapshot test (15)
**Evidence:** This is an open HANDOFF Track 2 item. `RefreshPricesHandler` persisting a `portfolio_daily_snapshots` row is tested only at the service layer.
**Fix:** Add one job-handler test with `test_pool()`. **Effort:** 1 hour.

### TD-12 — Frontend tests and component size (12)
**Evidence:** There are no unit tests; only Playwright exists. The largest components are `AdminModule.tsx` (593 lines), `LifeHorizonMinimap.tsx` (572), `SecurityPanel.tsx` (506) and `AiAssistantView.tsx` (500).
**Fix:** Add Vitest only for logic-heavy hooks and utilities, such as time-zone formatting and the admin query and step-up state. Split `AdminModule` per tab as part of the M17/M18 fixes. **Effort:** 1–2 days.

### TD-13 — Deferred edge cases (12)
These are HANDOFF Track 2 item 7:
- `SyncHealth` has no fallback when the Matrix sync timestamp fails to parse.
- The morning briefing window does not handle wrap-around near the hour boundary (`:58`–`:02`).

Each fix needs a regression test that fails first. **Effort:** 2–3 hours.

### TD-14 — `too_many_arguments` (9)
**Evidence:** 23 sites, mostly in repository insert functions (`finance/transactions.rs`, `portfolio/trades.rs`, `portfolio/audit.rs`, `provider_secrets.rs`, `knowledge/search.rs`) and chat adapters (`chat/llm.rs`, `chat/ocr`, `chat/voice`, `matrix/attachments.rs`).
**Fix:** Have repositories take the validated domain command or a `New…` struct that already exists at the boundary, and have chat adapters take a context struct. Do each site when its feature is next touched. **Effort:** spread over time.

### TD-15 — Quote taint blocks quote-based confirmation (9)
This is HANDOFF Track 2 item 2. Track the event IDs of real AI replies in a bounded store, so that quoting the bot's own AI reply does not taint the turn. Because this touches a security boundary, it needs a spec first. **Effort:** 1–2 days.

### TD-16 — Major-version lag (8)
Current versions: `axum 0.7` → 0.8, `utoipa 4` → 5, `tower-http 0.5` → 0.6, `thiserror 1` → 2, `chrono-tz 0.9`, `webpki-roots 0.26`.
Nothing is vulnerable today, but the gap grows with time. Upgrade axum, tower-http and utoipa together because they are coupled, and require byte-identical `openapi.json` or a reviewed diff. **Effort:** 2–4 days. Schedule it in its own branch, never mixed with a feature.

### TD-17 — Agent task stubs (8)
**Evidence:** In `domain/agent/task.rs`, the five built-in tasks carry `[TODO Extension]` doc comments. These break the AGENTS TODO format, which requires an issue number and a removal condition. The planned procedural pipelines, such as deterministic portfolio drift and TTS digests, are HANDOFF Track 2 item 3.
**Fix:** Do TD-01 first. Then either implement the pipelines or reword the comments to describe what the tasks actually do today. **Effort:** feature-sized.

---

## Phased remediation plan

Each phase is sized to run alongside feature work. Every item ships as its own Conventional Commit with the AGENTS §8 gates.

### Phase 0 — before merging `feat/config-management`
1. **TD-02:** finish, verify and commit the review remediation, and mark each finding in the review doc.
2. **TD-01:** enforce the scheduled-task tool whitelist, with tests on both the fresh and the resume path.

*Exit:* the review doc has no open HIGH findings; the gates are green.

### Phase 1 — next 1–2 weeks (about 2–3 days in total)
3. **TD-03:** Playwright in CI.
4. **TD-04, TD-06:** prune completed plans, fix doc drift, and point HANDOFF §6 at AGENTS §8.
5. **TD-09:** delete dead code and justify or drop the allows.
6. **TD-07:** move hashing out of `domain/` and forbid `sha2` in the architecture test.
7. **TD-08:** split `application/secrets/mod.rs`.
8. **TD-11:** price-refresh snapshot test.
9. **TD-05:** start with the admin module, typing its fakes and hooks against the generated `api.ts`, then make it the rule for every new module.

*Exit:* CI runs every gate listed in AGENTS §8 except the live/ignored tests; no file is over the cap; no unexplained `#[allow]` remains.

### Phase 2 — opportunistic, over the next month
10. **TD-10:** `reqwest 0.12` in its own branch.
11. **TD-13:** the edge-case fixes with regression tests.
12. **TD-12, TD-14:** whenever a feature touches the affected file, split the component, add hook tests, or replace the long argument list. Never as a separate sweep.
13. **TD-05:** migrate the remaining modules.

### Phase 3 — planned work items (each needs a spec in `docs/superpowers/specs/`)
14. **TD-15:** authentic-AI-reply tracking for quote confirmation.
15. **TD-16:** the axum, tower-http and utoipa major upgrade.
16. **TD-17:** procedural pipelines for the built-in agent tasks.

## Not counted as debt

- **The production cutover (HANDOFF Track 1)** is operator work, not code debt.
- **The Discord feature gaps and WebAuthn conditional UI** (HANDOFF Track 2 items 4–5) are roadmap features.
- **The 13 `#[ignore]` tests are intended.** They are the live-Postgres and migration-rehearsal tests; CI runs `live_postgres`.
- **The `unwrap()`/`expect()` counts are high, but the largest are in files dominated by `#[cfg(test)]` modules** (`merge.rs` 146, `link.rs` 93, `jobs.rs` 91). This audit did not sweep the production-only uses; the ones seen were on constants (for example `TaskId::new("…").expect("valid task id")`). A clippy `unwrap_used` pass restricted to non-test code would settle it.
