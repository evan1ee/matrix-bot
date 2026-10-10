# Tech-debt remediation, phases 0–1: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the phase 0 and phase 1 items of the 2026-10-10 tech-debt audit: land the config-management review fixes, enforce scheduled-task tool whitelists, then fix CI coverage, doc drift, layering, file size, dead code and the admin type drift.

**Architecture:** No new layers or abstractions. TD-01 adds one pure policy function to the domain registry and wires it through the existing `StepContext.allowed_tools` field. The rest of the work is deletions, small moves and CI configuration. Each audit item ships as its own Conventional Commit.

**Tech stack:** Rust (Axum, sqlx, tokio), PostgreSQL 18 test database, React 19 + TypeScript + Playwright, GitHub/Gitea Actions.

**Spec:** [`docs/superpowers/reviews/2026-10-10-tech-debt-audit.md`](../reviews/2026-10-10-tech-debt-audit.md). TD-02 also uses [`2026-10-10-config-management-review.md`](../reviews/2026-10-10-config-management-review.md).

**Out of scope:** Phase 2 items (TD-10 reqwest 0.12, TD-12, TD-13, TD-14, TD-05 for modules other than admin) and Phase 3 items (TD-15, TD-16, TD-17). Each needs its own plan, and Phase 3 items need a spec first.

## Global constraints

- Follow `AGENTS.md`. Hard rules that apply to every task:
  - Never run the bot (`cargo run`, `docker compose up`). The one allowed run is `cargo run --bin generate-openapi`.
  - Never contact a homeserver, LLM provider or other external service from a test.
  - Never read or modify `.env` or `data/`.
  - Never use bare `git stash`.
  - Never push, merge, open a PR or tag without the owner's explicit instruction.
- Point `TEST_DATABASE_URL` only at the local throwaway server (`pg18-test`).
- No file in `backend/src/` or `inference/src/` may exceed 500 production lines. Production lines are the lines before the first top-level `#[cfg(test)]`.
- Commits use Conventional Commits and end with the line `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Keep one logical change per commit, and never mix a refactor with a feature.
- In the same commit as its fix, flip each item's row in the audit register (`2026-10-10-tech-debt-audit.md`) to **Done**. That register is the progress log; `HANDOFF.md` points to it (Task 2).
- Migration and `.sqlx/` files stay LF. Do not let an editor convert them.

**Branches:**
- Tasks 1–2 run on `feat/config-management`. They are the merge gate for that branch.
- Tasks 3–10 run on a new branch, `chore/tech-debt-phase-1`. Create it from `master` if the owner has merged `feat/config-management` by then. Otherwise create it from `feat/config-management`.

## Review focus

These are the input classes most likely to bite and that no existing test covers. Each line has a test in the task that owns the code.

1. **A scheduled run whose task is no longer registered**, for example because it was renamed or removed. The run must get **no** tools (fail closed), not every tool. Covered by Task 2, Step 1, `unknown_scheduled_task_gets_no_tools`.
2. **A built-in whitelist that names a tool which does not exist.** This silently leaves the task with no tools. Covered by Task 2, Step 1, `every_builtin_whitelist_names_registered_tools`.
3. **An approved held call, on a scheduled run, for a tool outside the whitelist.** For example, an approval created before this fix shipped. The call must be refused on the resume path. Covered by Task 2, Step 2, `resume_refuses_an_approved_call_outside_the_task_whitelist`.
4. **User-started runs must keep full tool access.** Over-restricting them is the likely regression. Covered by Task 2, Step 1, `user_run_is_not_restricted`, plus the existing approval tests in `interface/jobs/agent/mod.rs`.
5. **The hashing move changes a stored digest.** If it did, every session cookie, webhook token and portfolio audit chain would stop matching. Covered by the pinned-digest tests in Task 6, Steps 1 and 4.

---

## Phase 0 — on `feat/config-management`

### Task 1: TD-02 — verify, triage and commit the config-review fixes

The working tree holds 66 uncommitted files, mostly the owner's remediation of the config-management review. This task does **not** rewrite that work. It verifies the work, closes any HIGH finding that is still open, records a status for every finding, and commits in logical groups.

**Files:**
- Modify: `docs/superpowers/reviews/2026-10-10-config-management-review.md`, adding a status column to §3 and §4.
- Modify: whichever files a still-open HIGH finding needs. The fix for each one is in the review's "Fix" column.
- Commit: all 66 working-tree files.

**Interfaces:** none produced. Tasks 7 and 9 build on the committed result of `application/secrets/mod.rs` and `frontend/src/modules/admin/`.

- [ ] **Step 1: Snapshot the tree before touching anything**

```bash
git status --short > "$TMPDIR/td02-status-before.txt"
git diff --stat > "$TMPDIR/td02-diffstat-before.txt"
```

Use the scratchpad directory for `$TMPDIR`. Do not stash anything.

- [ ] **Step 2: Check each blocker against the code**

Run each check and write the result (Fixed, Open or Partial) into a scratch list. Spot checks during planning suggested the expected results shown below. Confirm them; do not assume.

| ID | Check | Expected if fixed |
|---|---|---|
| B1 | `grep -n "got: {part}\|Expected version" backend/src/infrastructure/crypto/aes_gcm.rs` | no hit; a test asserts the error text has no key fragment |
| B2 | `grep -n "without_url" backend/src/infrastructure/inference/testers/mod.rs` | a hit on every reqwest error path |
| B3 | Read `backend/src/llm/resolved_config.rs`. Check that the base URL and the key are resolved together, and that a user-supplied base URL never receives a shared key. | there is a test for the mismatched-provider case and one for the user-base-URL-without-key case |
| B4 | `cargo test -p matrix-bot test_uncustomized_snapshot_preserves_env_model` | PASS |
| B5 | Read `backend/src/events/listener.rs`. Check that no `select!` branch can drop the `listen` future, and that reload errors are logged. | yes |
| B6 | `grep -n "entries" frontend/src/modules/admin/AdminModule.tsx` | it reads `.entries` |
| B7 | `grep -n "useSensitiveAction" frontend/src/modules/admin/useAdmin.ts` | every mutation uses it, and the queries render `isError` |
| B8 | `grep -rn "ADMIN_USERS\|is_admin_in_allowlist\|SECRETS_ENCRYPTION_KEY_OLD" backend/src .env.example docs` | no hit, or a hit backed by implemented and tested code |

For M1–M18, read each finding's file reference and record a status the same way.

- [ ] **Step 3: Close each open HIGH finding with a regression test first**

For each blocker still marked Open or Partial:

1. Write the test the review's "Fix" column describes.
2. Run it and confirm it fails.
3. Apply the fix.
4. Confirm the test passes.

A HIGH finding may **not** be deferred without the owner's approval. If one needs a decision from review §7 (the open questions), stop and ask the owner.

Leave MEDIUM findings that are not fixed as Deferred, each with a one-line reason.

- [ ] **Step 4: Record the status of every finding in the review doc**

Add a `Status` column to the tables in §3 and §4 of `2026-10-10-config-management-review.md`. Values are `Fixed`, `Deferred: <reason>` or `Won't fix: <reason>`. In §7, under each open question, write down the decision the code now reflects. For example, the admin bootstrap is SQL-only if B8 was resolved by deleting the dead code.

- [ ] **Step 5: Run the full gate set**

```bash
cargo fmt --all -- --check
cargo check -p matrix-bot --all-targets
cargo clippy -p matrix-bot --all-targets -- -D warnings
cargo clippy -p matrix-bot-inference --all-targets -- -D warnings
cargo test -p matrix-bot --all-targets
cargo test -p matrix-bot live_postgres -- --ignored --test-threads=1
cargo run --bin generate-openapi
git diff --exit-code backend/openapi.json
cargo sqlx prepare --workspace --check -- -p matrix-bot --all-targets
npm --prefix frontend run generate-api
npm --prefix frontend run build
npm --prefix frontend run test:e2e
git diff --check
```

The `openapi.json` diff check must pass **after** you have committed the regenerated file. The file already appears as modified in the working tree, so expect a diff now. Confirm the diff only reflects intended DTO changes, then keep it.

Expected result: every command exits 0. Fix failures before going on. Re-run the full suite after every fix.

- [ ] **Step 6: Commit in logical groups**

`git add -p` is interactive and not available here. Stage whole files instead. If a file mixes groups, commit it with the group that owns most of its hunks.

```bash
# 1. Secret handling: B1, B2, M5–M8, M16
git add backend/src/infrastructure/crypto/aes_gcm.rs backend/src/infrastructure/inference/testers/mod.rs \
  backend/src/application/secrets/mod.rs backend/src/domain/secrets/mod.rs \
  backend/src/infrastructure/postgres/repositories/provider_secrets.rs backend/src/interface/http/admin/secrets.rs
git commit -m "fix(secrets): keep secrets out of errors, logs and audit; trim and validate once

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"

# 2. LLM credential resolution: B3, B4, and the consumers that read it
git add backend/src/llm/ backend/src/application/knowledge/ backend/src/interface/chat/ \
  backend/src/infrastructure/inference/ocr backend/src/infrastructure/inference/stt backend/src/infrastructure/inference/tts \
  backend/src/interface/jobs/media.rs backend/src/interface/http/ocr.rs backend/src/interface/http/voice.rs
git commit -m "fix(llm): resolve provider, base URL and key together; env AI_MODEL wins over unset settings

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"

# 3. Listener and reload: B5, M2, M4, M13
git add backend/src/events/ backend/src/app.rs backend/src/main.rs backend/src/application/config/ backend/src/application/mod.rs
git commit -m "fix(config): never cancel LISTEN from the TTL tick; load snapshot at boot; serialise reloads

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"

# 4. Registry, validation, audit and schema: M3, M10, M12, M14, M15
git add backend/src/domain/config/ backend/migrations/0015_configuration_and_secrets.sql backend/.sqlx \
  backend/src/domain/accounts/mod.rs backend/src/application/accounts/ backend/src/application/identity/link.rs \
  backend/src/config/validate.rs backend/src/interface/http/admin/ backend/src/interface/http/auth/ backend/src/interface/http/mod.rs
git commit -m "fix(config): validate persisted settings, split registry, tighten URL validation and audit

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"

# 5. Frontend admin console and contract: B6, B7, M17, M18
git add backend/openapi.json frontend/
git commit -m "fix(admin): read the paged audit shape, step-up and error states, accessible modals

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"

# 6. Docs: B8, M11, and the review status
git add .env.example README.md docs/
git commit -m "docs(config): match docs to the shipped admin bootstrap and schema; record review status

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

Before each commit, run `git status --short` and check that no unrelated file is staged. After the last commit, `git status --short` must be empty, apart from any file you can name and justify. One notable case: migration `0015` is edited in place. That is allowed only because `git cat-file -e master:backend/migrations/0015_configuration_and_secrets.sql` fails, meaning the file is not on `master`. Confirm that it still fails, and ask the owner whether any environment has applied `0015`. If one has, the change must move to a new migration instead.

- [ ] **Step 7: Re-run the gates on the committed tree**

Re-run Step 5. Expected: all green, and `git diff --exit-code backend/openapi.json` now passes.

---

### Task 2: TD-01 — enforce scheduled-task tool whitelists

**Files:**
- Modify: `backend/src/domain/agent/task.rs`. Add `ScheduledTaskRegistry::tool_policy`, its tests, and reword the "Stubbed as TODO Extensions" banner.
- Modify: `backend/src/application/agent/tasks.rs`. Add `AgentService::tool_policy`.
- Modify: `backend/src/interface/jobs/agent/mod.rs`, around line 159. Pass the policy, and add the job tests.
- Modify: `backend/src/interface/jobs/agent/resume.rs`, around line 131. Pass the policy.
- Modify: `backend/src/llm/agent/engine.rs`, lines 252–260. Update the doc comment.
- Modify: `backend/src/llm/agent/sandbox.rs`. Add the whitelist-names test.
- Modify: `docs/superpowers/HANDOFF.md`. Add a Track 2 pointer to the audit.
- Modify: `docs/superpowers/reviews/2026-10-10-tech-debt-audit.md`. Mark TD-01 and TD-02 Done.

**Interfaces:**
- Produces: `ScheduledTaskRegistry::tool_policy(&self, run: &AgentRun) -> Option<Vec<String>>`. `None` means unrestricted, which applies to user-started runs. `Some(list)` restricts the run to `list`. `Some(vec![])` means no tools.
- Produces: `AgentService::tool_policy(&self, run: &AgentRun) -> Option<Vec<String>>`, which delegates to the registry.
- Consumes: `StepContext.allowed_tools: Option<&[String]>` (existing), `sandbox::is_tool_allowed`, `sandbox::filter_tool_definitions`.

- [ ] **Step 1: Write the domain and sandbox tests (RED)**

In `backend/src/domain/agent/task.rs`, inside `mod tests`, add:

```rust
    use crate::domain::agent::AgentRun;

    fn run_for(task_name: &str, scheduled: bool) -> AgentRun {
        let mut run = AgentRun::new(
            AccountId::from_seed("acct_task_policy"),
            task_name,
            "Goal",
            None,
            None,
            Utc::now(),
        )
        .unwrap();
        run.scheduled_task_id = scheduled.then(Uuid::new_v4);
        run
    }

    #[test]
    fn user_run_is_not_restricted() {
        let registry = ScheduledTaskRegistry::with_builtins();
        assert_eq!(registry.tool_policy(&run_for("user_goal", false)), None);
    }

    #[test]
    fn scheduled_run_is_held_to_its_task_whitelist() {
        let registry = ScheduledTaskRegistry::with_builtins();
        assert_eq!(
            registry.tool_policy(&run_for("habit_consistency_review", true)),
            Some(vec!["get_my_habits".to_string(), "get_daily_log".to_string()])
        );
    }

    #[test]
    fn unknown_scheduled_task_gets_no_tools() {
        let registry = ScheduledTaskRegistry::with_builtins();
        assert_eq!(
            registry.tool_policy(&run_for("removed_task", true)),
            Some(Vec::new())
        );
        assert_eq!(
            registry.tool_policy(&run_for("Not A Valid Id!", true)),
            Some(Vec::new())
        );
    }
```

In `backend/src/llm/agent/sandbox.rs`, inside `mod tests`, add:

```rust
    #[test]
    fn every_builtin_whitelist_names_registered_tools() {
        let registered: Vec<String> = crate::llm::tools::ToolRegistry::get_tool_definitions()
            .into_iter()
            .map(|t| t.function.name)
            .collect();
        for task in crate::domain::agent::ScheduledTaskRegistry::with_builtins().list() {
            for tool in task.allowed_tools().unwrap_or_default() {
                assert!(
                    registered.contains(&tool),
                    "task {} whitelists unknown tool {tool}",
                    task.task_id()
                );
            }
        }
    }
```

- [ ] **Step 2: Write the job tests (RED)**

In `backend/src/interface/jobs/agent/mod.rs`, inside `mod tests`, add a fixture and two tests:

```rust
    /// A scheduled run of `habit_consistency_review`, started the way the
    /// scheduler starts it. That task may only call `get_my_habits` and
    /// `get_daily_log`.
    async fn scheduled_fixture(
        seed: &str,
        replies: Vec<serde_json::Value>,
    ) -> (
        AgentExecuteRunHandler,
        AppServices,
        AccountId,
        AgentRunId,
        crate::llm::test_support::Seen,
    ) {
        use crate::llm::test_support::*;
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let services = AppServices::new(&pool);
        let owner = AccountId::from_seed(seed);
        crate::db::pool::seed_account(&pool, &owner).await;
        let (url, seen) = scripted_replies(replies).await;
        let handler = AgentExecuteRunHandler::new(
            services.clone(),
            pool.clone(),
            FakeOutbound::connected(),
            ReActEngine::from_proxy(None),
            test_config(url),
        );
        services
            .agent
            .schedule_task(
                &owner,
                crate::domain::agent::TaskId::new("habit_consistency_review").unwrap(),
                "0 20 * * 0",
                serde_json::json!({}),
                now_at_storage_precision() - chrono::Duration::minutes(1),
            )
            .await
            .unwrap();
        services.agent.enqueue_due_scheduled_tasks().await.unwrap();
        let run = services
            .agent
            .get_active_run(&owner)
            .await
            .unwrap()
            .expect("the scheduler started a run");
        assert!(run.scheduled_task_id.is_some());
        (handler, services, owner, run.id, seen)
    }

    #[tokio::test]
    async fn scheduled_run_cannot_call_a_tool_outside_its_whitelist() {
        use crate::llm::test_support::{text_reply, tool_reply};
        let (handler, services, owner, run_id, seen) = scheduled_fixture(
            "agent-whitelist-fresh",
            vec![
                tool_reply("add_todo", serde_json::json!({"text": "injected"})),
                text_reply("unused"),
            ],
        )
        .await;

        run_job(&handler, owner, run_id).await;

        let todos = services.todos.list(&owner, None, None, 10, 0).await.unwrap();
        assert!(todos.is_empty(), "the call outside the whitelist never ran");
        let run = services.agent.get_run_internal(run_id).await.unwrap();
        assert_eq!(run.status, RunStatus::Failed);
        assert!(
            run.error.as_deref().unwrap_or_default().contains("not allowed by task policy"),
            "{:?}",
            run.error
        );
        let requests = seen.lock().unwrap();
        let offered: Vec<&str> = requests[0]["tools"]
            .as_array()
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|t| t["function"]["name"].as_str())
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            offered.iter().all(|n| ["get_my_habits", "get_daily_log"].contains(n)),
            "only whitelisted tools are offered: {offered:?}"
        );
    }

    #[tokio::test]
    async fn resume_refuses_an_approved_call_outside_the_task_whitelist() {
        use crate::llm::test_support::text_reply;
        let (handler, services, owner, run_id, _seen) =
            scheduled_fixture("agent-whitelist-resume", vec![text_reply("unused")]).await;
        // A held, approved deposit, e.g. one approved before the whitelist
        // was enforced.
        let call = serde_json::json!({"amount": "100", "currency": "USD"});
        let held = AgentStep::new(
            run_id,
            1,
            None,
            Some("deposit_portfolio_cash".to_string()),
            Some(call.clone()),
            None,
            StepStatus::Blocked,
            0,
            now_at_storage_precision(),
        );
        services.agent.record_step(run_id, &held).await.unwrap();
        let approval = services
            .agent
            .require_approval(run_id, "deposit_portfolio_cash", call)
            .await
            .unwrap();
        services.agent.approve_action(&owner, approval.id).await.unwrap();

        run_job(&handler, owner, run_id).await;

        assert!(cash_minor(&services, &owner).await.is_empty(), "nothing ran");
        let run = services.agent.get_run_internal(run_id).await.unwrap();
        assert_eq!(run.status, RunStatus::Failed);
        assert!(
            run.error.as_deref().unwrap_or_default().contains("not allowed by task policy"),
            "{:?}",
            run.error
        );
    }
```

If `chrono` is not yet imported in the test module, refer to it by its full path as shown above.

- [ ] **Step 3: Run the new tests to verify they fail**

```bash
cargo test -p matrix-bot domain::agent::task::tests
cargo test -p matrix-bot interface::jobs::agent::tests::scheduled_run_cannot_call
cargo test -p matrix-bot interface::jobs::agent::tests::resume_refuses
cargo test -p matrix-bot every_builtin_whitelist_names_registered_tools
```

Expected:
- The task tests do not compile, because `tool_policy` is not defined yet.
- The two job tests FAIL: the todo is created or the deposit runs.
- `every_builtin_whitelist_names_registered_tools` either passes or names a wrong tool. If it names one, the fix belongs in the whitelist in `task.rs`: change the name to the registered tool that does the same thing. Do not change the test.

- [ ] **Step 4: Implement the policy in the domain**

In `backend/src/domain/agent/task.rs`, add `use super::AgentRun;` to the imports. Then add this method inside `impl ScheduledTaskRegistry`:

```rust
    /// The tools `run` may call.
    ///
    /// A user-started run is not restricted here (`None`): sensitive tools
    /// still need the user's approval. A scheduled run runs unattended on
    /// prompts that can carry untrusted content, so it is held to its task's
    /// whitelist. If its task is no longer registered, or its stored name is
    /// not a valid id, it gets no tools at all rather than every tool.
    pub fn tool_policy(&self, run: &AgentRun) -> Option<Vec<String>> {
        if run.scheduled_task_id.is_none() {
            return None;
        }
        let task = TaskId::new(&run.task_name)
            .ok()
            .and_then(|id| self.get(&id));
        match task {
            Some(task) => task.allowed_tools(),
            None => Some(Vec::new()),
        }
    }
```

Change the banner comment `// Built-in Task Plugins (Stubbed as TODO Extensions)` to `// Built-in task plugins. Each one is a goal prompt plus a tool whitelist.`. Leave the per-task `[TODO Extension]` doc comments for TD-17.

- [ ] **Step 5: Wire it through the service and both job paths**

In `backend/src/application/agent/tasks.rs`, add `AgentRun` to the existing `crate::domain::agent` import if it is not there. Then add inside `impl AgentService`:

```rust
    /// The tool whitelist the run executes under (see
    /// `ScheduledTaskRegistry::tool_policy`).
    pub fn tool_policy(&self, run: &AgentRun) -> Option<Vec<String>> {
        self.registry.tool_policy(run)
    }
```

In `backend/src/interface/jobs/agent/mod.rs`, in `run()`, right after the `run.status.is_terminal()` early return:

```rust
        let allowed_tools = self.services.agent.tool_policy(&run);
```

In the `StepContext` literal, replace `allowed_tools: None,` with:

```rust
                allowed_tools: allowed_tools.as_deref(),
```

In `backend/src/interface/jobs/agent/resume.rs`, in `resume_approved`, right before `let step_ctx = StepContext {`:

```rust
        let allowed_tools = self.services.agent.tool_policy(run);
```

In that `StepContext` literal, replace `allowed_tools: None,` with `allowed_tools: allowed_tools.as_deref(),`. Do not add a parameter, because `resume_approved` would then trip `too_many_arguments`.

In `backend/src/llm/agent/engine.rs`, replace the last two sentences of the `execute_approved` doc comment, "The whitelist check below is a no-op today: the job passes `allowed_tools: None` (a HANDOFF follow-up wires task whitelists in).", with:

```rust
    /// The whitelist is checked again here: a scheduled run's held call must
    /// still be one its task may make, even after the user approved it.
```

- [ ] **Step 6: Run the tests to verify they pass**

```bash
cargo test -p matrix-bot domain::agent::task::tests
cargo test -p matrix-bot interface::jobs::agent::
cargo test -p matrix-bot llm::agent::
```

Expected: all PASS. That includes the existing approval tests, which prove that user runs are unchanged.

- [ ] **Step 7: Point HANDOFF at the audit and mark the items Done**

In `docs/superpowers/HANDOFF.md` §4 Track 2, add one line: `- Tech-debt register and phased plan: [reviews/2026-10-10-tech-debt-audit.md](reviews/2026-10-10-tech-debt-audit.md); execution plan: [plans/2026-10-11-tech-debt-phase-0-1.md](plans/2026-10-11-tech-debt-phase-0-1.md).`

Remove the Track 2 line that says scheduled-task whitelists are not wired in, if there is one.

In the audit register, mark TD-01 and TD-02 **Done**.

- [ ] **Step 8: Gates and commit**

```bash
cargo fmt --all -- --check
cargo clippy -p matrix-bot --all-targets -- -D warnings
cargo test -p matrix-bot --all-targets
git add backend/src/domain/agent/task.rs backend/src/application/agent/tasks.rs \
  backend/src/interface/jobs/agent/ backend/src/llm/agent/ \
  docs/superpowers/HANDOFF.md docs/superpowers/reviews/2026-10-10-tech-debt-audit.md
git commit -m "fix(agent): hold scheduled runs to their task's tool whitelist

Both job entry points passed allowed_tools: None, so an unattended run
could call any tool. A run whose task is no longer registered now gets
no tools (fail closed); user-started runs are unchanged.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

**Phase 0 exit:** the review doc shows no open HIGH finding, and all gates are green. Ask the owner whether to push `feat/config-management` and open the PR on Gitea. Do not do either without their approval.

---

## Phase 1 — on `chore/tech-debt-phase-1`

```bash
git switch -c chore/tech-debt-phase-1 master   # or feat/config-management if it is not merged yet
```

### Task 3: TD-03 and part of TD-05 — Playwright and the API-types drift check in CI

**Files:**
- Modify: `.github/workflows/ci.yml`.
- Modify: `AGENTS.md` §8, last paragraph (the CI description).

**Interfaces:** none.

- [ ] **Step 1: Add the drift check to the existing `frontend` job**

In `.github/workflows/ci.yml`, after the `Install dependencies` step of the `frontend` job, add:

```yaml
      # The generated client types must match the committed OpenAPI spec, or
      # the modules and e2e fakes are typed against an API that does not exist.
      - name: Generated API types match openapi.json
        run: |
          git config --global --add safe.directory "$GITHUB_WORKSPACE"
          npm --prefix frontend run generate-api
          git diff --exit-code frontend/src/types/api.ts
```

- [ ] **Step 2: Add the e2e job**

Add this job next to the `frontend` job:

```yaml
  frontend-e2e:
    name: Frontend e2e (Playwright)
    runs-on: ubuntu-latest
    # The official image carries Node and the browser with its system
    # libraries, so the job neither needs sudo nor assumes the runner image:
    # the same reason the backend job is pinned into a container. Its tag must
    # match @playwright/test in frontend/package-lock.json.
    container: mcr.microsoft.com/playwright:v1.62.1-noble
    steps:
      - uses: actions/checkout@v4

      - name: Install dependencies
        run: npm --prefix frontend ci

      # The specs run against e2e/fake-api.ts: no backend, no database.
      - name: Playwright
        env:
          CI: "true"
        run: npm --prefix frontend run test:e2e

      - name: Upload traces on failure
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: playwright-traces
          path: frontend/test-results/
          retention-days: 7
```

If the Gitea runner rejects `upload-artifact@v4`, which older Gitea versions do, use `actions/upload-artifact@v3` with the same inputs.

- [ ] **Step 3: Check it locally**

```bash
npm --prefix frontend run generate-api
git diff --exit-code frontend/src/types/api.ts
CI=true npm --prefix frontend run test:e2e
```

Expected: no diff, and every spec passes with `retries: 2` under `CI`. A spec that only passes on retry is flaky. Record it in the commit body; do not hide it.

- [ ] **Step 4: Fix the CI description in AGENTS §8**

In `AGENTS.md`, replace the sentence "CI (`.github/workflows/ci.yml`) runs `backend`, `inference` and `frontend` jobs; clippy is a required local gate but not yet a CI job." with:

"CI (`.github/workflows/ci.yml`) runs `backend` (fmt, clippy, sqlx check, tests), `inference`, `frontend` (generated-types drift check, build) and `frontend-e2e` (Playwright against the fake API)."

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/ci.yml AGENTS.md docs/superpowers/reviews/2026-10-10-tech-debt-audit.md
git commit -m "ci: run Playwright e2e and check generated API types against openapi.json

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

Mark TD-03 Done in the audit register before committing. TD-05 stays open until Task 9.

---

### Task 4: TD-04 and TD-06 — prune finished plans and specs, fix doc drift

**Files:**
- Delete: finished plans in `docs/superpowers/plans/`, and specs that no live document references.
- Modify: `docs/superpowers/HANDOFF.md` §6.

**Interfaces:** none.

- [ ] **Step 1: List the specs still referenced by a live document**

```bash
grep -rhoE "specs/[0-9a-z-]+\.md" AGENTS.md README.md docs/*.md docs/superpowers/HANDOFF.md docs/superpowers/runbooks/ | sort -u
```

Keep exactly the specs this prints. During planning it printed `2026-09-24-postgres-foundation-design.md` and `2026-09-29-job-queue-design.md`. Re-run it rather than trusting that list.

- [ ] **Step 2: Delete the finished plans and the unreferenced specs**

Delete every file in `docs/superpowers/plans/` except this plan. All of their work is reported as shipped in HANDOFF §2, and `2026-10-09-configuration-management.md` is shipped by Task 1.

Delete every spec in `docs/superpowers/specs/` that Step 1 did not print.

```bash
git rm $(ls docs/superpowers/plans/*.md | grep -v 2026-10-11-tech-debt-phase-0-1.md)
git rm <each unreferenced spec>
```

Then check that no link now points to a deleted file:

```bash
grep -rnE "plans/2026-|specs/2026-" AGENTS.md README.md Design.md docs/ | grep -v "2026-10-11-tech-debt-phase-0-1"
```

Expected: only the kept specs. Fix or remove any other hit. A link into a deleted plan inside HANDOFF should become a plain sentence of what was decided.

Also delete `docs/superpowers/reviews/2026-10-10-config-management-review.md` if Task 1 left every finding Fixed or Won't fix. If any finding is Deferred, keep the file until it is closed.

- [ ] **Step 3: Make HANDOFF §6 point to AGENTS §8**

In `docs/superpowers/HANDOFF.md` §6, replace the copied command list, which uses `cargo clippy --workspace …` and has drifted from AGENTS, with:

`The verification gates are listed once, in [AGENTS.md §8](../../AGENTS.md#8-verification). Run them from the repository root.`

Keep any HANDOFF-specific notes in §6 that are not part of the command list, such as the latest results.

- [ ] **Step 4: Commit**

Mark TD-04 and TD-06 Done in the audit register. TD-06's M11 part is closed by Task 1.

```bash
git add -A docs/ AGENTS.md
git commit -m "docs: delete finished plans and specs; point HANDOFF gates at AGENTS §8

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: TD-09 — delete dead code and unexplained `#[allow]`s

**Files:**
- Modify: `backend/src/interface/http/reminders.rs:23-80`.
- Modify: `backend/src/interface/chat/mod.rs:190-196`, plus every `CommandHelp { … }` literal.
- Modify: `backend/src/interface/http/auth/matrix.rs:14-15,133-138`.
- Modify: `backend/src/interface/http/agent.rs:14-15`.
- Modify: `backend/src/interface/http/storage.rs:20-21`.
- Modify: `inference/src/ocr/mod.rs:3`, `inference/src/stt/mod.rs:6,9`, `inference/src/tts/mod.rs:4`.

**Interfaces:** none. Public HTTP contracts must not change, and `backend/openapi.json` must be byte-identical afterwards.

- [ ] **Step 1: Delete the four dead reminder helpers**

In `backend/src/interface/http/reminders.rs`, delete `RecurrencePattern::as_str`, `RecurrencePattern::from_str_or_default`, `ReminderStatus::as_str` and `ReminderStatus::from_str_or_default`, together with their `#[allow(dead_code)]`. Delete an `impl` block if it is left empty.

- [ ] **Step 2: Delete the unread fields**

- `backend/src/interface/http/auth/matrix.rs`: delete the `#[serde(default)] #[allow(dead_code)] device_id: Option<String>,` field. Serde ignores unknown fields unless the struct says `deny_unknown_fields`, so check that it does not.
- `backend/src/interface/chat/mod.rs`: run `grep -rn "\.name\b" backend/src | grep -i help` to see whether `CommandHelp.name` is read anywhere.
  - If it is not read, delete the field and the `name:` line in every `CommandHelp { … }` literal (`grep -rn "CommandHelp {" backend/src`).
  - If it is read, only drop the `#[allow]`.

- [ ] **Step 3: Drop the `unused_imports` allows on `ApiErrorResponse`**

In `agent.rs`, `storage.rs` and `auth/matrix.rs`, delete the `#[allow(unused_imports)]` line. Then:

```bash
cargo check -p matrix-bot --all-targets 2>&1 | grep -A3 "unused import"
```

If a file now warns, the import is used only inside `#[utoipa::path]`. In that case, delete the import and write the type in the attribute as `crate::interface::http::ApiErrorResponse`, which is re-exported by `interface/http/mod.rs:71`.

- [ ] **Step 4: Narrow the inference re-exports**

Delete the four `#[allow(unused_imports)]` lines in `inference/src/{ocr,stt,tts}/mod.rs`. Then:

```bash
cargo check -p matrix-bot-inference --all-targets 2>&1 | grep -B2 -A6 "unused import"
```

Remove each name the warning lists from its `pub use`. Leave `inference/src/ocr/paddle.rs:36` alone, because it already documents why it exists.

- [ ] **Step 5: Gates**

```bash
cargo fmt --all -- --check
cargo clippy -p matrix-bot --all-targets -- -D warnings
cargo clippy -p matrix-bot-inference --all-targets -- -D warnings
cargo test -p matrix-bot --all-targets
cargo test -p matrix-bot-inference --all-targets
cargo run --bin generate-openapi
git diff --exit-code backend/openapi.json
grep -rn "#\[allow(dead_code)\]\|#\[allow(unused_imports)\]" backend/src inference/src
```

Expected: everything green, no `openapi.json` diff, and the last grep prints only `inference/src/ocr/paddle.rs:36`.

- [ ] **Step 6: Commit**

Mark TD-09 Done in the audit register.

```bash
git add backend/src inference/src docs/superpowers/reviews/2026-10-10-tech-debt-audit.md
git commit -m "refactor: delete dead reminder helpers and unread fields; drop unexplained allows

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: TD-07 — move SHA-256 out of `domain/` and forbid it there

**Files:**
- Modify: `backend/src/utils/secure.rs`. Add `sha256_hex` and its test.
- Modify: `backend/src/domain/accounts/session.rs:23,84-92`.
- Modify: `backend/src/domain/webhooks/mod.rs:7,128-137`.
- Modify: `backend/src/domain/portfolio/audit.rs:5,40-81`.
- Modify: `backend/tests/architecture.rs:211-229`.

**Interfaces:**
- Produces: `crate::utils::secure::sha256_hex(bytes: &[u8]) -> String`, which returns 64 lowercase hex characters.
- The signatures of `hash_token` (in both modules) and `compute_entry_hash` do not change.

**Ruling:** `domain/` may call `crate::utils`, as `domain/webhooks/renderer.rs` already does with `utils::html`. The layer rule is about external crates. One helper also removes the two identical hex folds.

- [ ] **Step 1: Pin the session digest (it must pass before and after)**

In `backend/src/domain/accounts/session.rs`, inside `mod tests` (create the module if there is none), add:

```rust
    /// Stored session ids are this digest; changing it logs everyone out.
    #[test]
    fn hash_token_digest_is_pinned() {
        assert_eq!(
            hash_token("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
```

```bash
cargo test -p matrix-bot domain::accounts::session::tests::hash_token_digest_is_pinned
```

Expected: PASS. This test guards the refactor; it is not a RED test.

- [ ] **Step 2: Forbid `sha2::` in `domain/` (RED)**

In `backend/tests/architecture.rs`, `domain_depends_on_nothing_outside_itself`, add `"sha2::",` after `"webauthn_rs",`. Use the needle `sha2::`, not `sha2`, because `sha2` would also match `sha256_hex`.

```bash
cargo test -p matrix-bot --test architecture domain_depends_on_nothing_outside_itself
```

Expected: FAIL, listing `session.rs`, `portfolio/audit.rs` and `webhooks/mod.rs`.

- [ ] **Step 3: Add the helper**

In `backend/src/utils/secure.rs`, after `constant_time_eq`:

```rust
/// Lowercase hex SHA-256 of `bytes`. The crate spells this out once, so
/// `domain/` can hash a token or an audit record without depending on a
/// crypto crate itself.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut acc, byte| {
            let _ = write!(acc, "{byte:02x}");
            acc
        })
}
```

Then add this to its `mod tests`:

```rust
    #[test]
    fn sha256_hex_matches_the_fips_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
```

- [ ] **Step 4: Use it in the three domain files**

1. **`session.rs` and `webhooks/mod.rs`:** delete `use sha2::{Digest, Sha256};`. Replace the body of each `hash_token` with:

   ```rust
       crate::utils::secure::sha256_hex(token.as_bytes())
   ```

2. **`portfolio/audit.rs`:** delete `use sha2::{Digest, Sha256};`. Replace everything from `let mut hasher = Sha256::new();` to the end of `compute_entry_hash` with:

   ```rust
       // The colon-joined preimage is the stored chain's format; the pinned
       // digest test below fails if a single byte of it changes.
       let preimage = format!(
           "{prev_hash}:{owner_id}:{action}:{}:{}:{}:{amount_minor}:{currency}:\
            {currency_exponent}:{}:{}:{created_at_epoch_ms}",
           symbol.unwrap_or(""),
           quantity_minor.unwrap_or(0),
           price_minor.unwrap_or(0),
           realized_pl_minor.unwrap_or(0),
           note.unwrap_or(""),
       );
       crate::utils::secure::sha256_hex(preimage.as_bytes())
   ```

   The `\` line continuation inside the format string removes the newline and the leading whitespace of the next line, so the preimage contains no spaces. The pinned test proves it.

- [ ] **Step 5: Run the guards and the architecture test**

```bash
cargo test -p matrix-bot domain::portfolio::audit::tests
cargo test -p matrix-bot domain::accounts::session::tests
cargo test -p matrix-bot domain::webhooks::
cargo test -p matrix-bot utils::secure::
cargo test -p matrix-bot --test architecture
```

Expected: all PASS, including `test_compute_entry_hash_matches_pre_migration_pinned_digest` and `hash_token_digest_is_pinned`.

- [ ] **Step 6: Full suite, then commit**

Mark TD-07 Done in the audit register.

```bash
cargo clippy -p matrix-bot --all-targets -- -D warnings
cargo test -p matrix-bot --all-targets
git add backend/src/utils/secure.rs backend/src/domain backend/tests/architecture.rs docs/superpowers/reviews/2026-10-10-tech-debt-audit.md
git commit -m "refactor(domain): hash through utils::secure; forbid sha2 in domain/

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: TD-08 — split `application/secrets/mod.rs` under the 500-line cap

**Files:**
- Modify: `backend/src/application/secrets/mod.rs`. It keeps `SecretService`, the constructors, the reads (`list_metadata`, `is_configured`, `get_metadata_by_id`, `get_decrypted_secret`), the `From<CryptoError>`/`From<SecretError>` impls and the existing tests.
- Create: `backend/src/application/secrets/mutations.rs`, with `store_secret`, `replace_secret` and `delete_secret` in an `impl SecretService` block.
- Create: `backend/src/application/secrets/credential_check.rs`, with `TestResultDto`, `test_credential` and `scrub_secret_from_error`.

**Interfaces:**
- No path visible from outside changes. `mod.rs` adds `mod credential_check; mod mutations; pub use credential_check::TestResultDto;`, so `crate::application::secrets::TestResultDto` still resolves.

This is a pure move with no behaviour change. It follows the `application/agent/{lifecycle,approvals,tasks}.rs` pattern of several `impl AgentService` blocks.

- [ ] **Step 1: Record the baseline**

```bash
awk '/^#\[cfg\(test\)\]/{print NR-1; exit}' backend/src/application/secrets/mod.rs
cargo test -p matrix-bot application::secrets:: 2>&1 | tail -3
```

Note the production line count (514 at audit time, possibly different after Task 1) and the number of passing tests.

- [ ] **Step 2: Move the code**

1. Move the three mutation methods, unchanged, into `mutations.rs`:

   ```rust
   //! Writes to the vault: store, replace and delete, each audited and
   //! published in the same transaction as the change.

   use super::SecretService;
   // …only the imports these three methods use, moved from mod.rs…

   impl SecretService {
       // store_secret, replace_secret, delete_secret: moved verbatim
   }
   ```

2. Move `TestResultDto`, `test_credential` (in its own `impl SecretService` block) and the free function `scrub_secret_from_error`, unchanged, into `credential_check.rs`, with the doc header `//! Probing a stored credential against its provider.`

3. In `mod.rs`, delete the moved code. Add the two `mod` lines and the `pub use`. Remove imports that are now unused.

4. Keep the tests in `mod.rs`. If one calls `scrub_secret_from_error` directly, make that function `pub(super)`.

- [ ] **Step 3: Verify the move**

```bash
cargo check -p matrix-bot --all-targets
cargo test -p matrix-bot application::secrets::
for f in backend/src/application/secrets/*.rs; do printf "%s " "$f"; awk '/^#\[cfg\(test\)\]/{print NR-1; f=1; exit} END{if(!f) print NR}' "$f"; done
cargo run --bin generate-openapi
git diff --exit-code backend/openapi.json
```

Expected:
- The same number of tests pass as in Step 1.
- Every file is under 500 production lines; `mod.rs` should be about 250.
- `openapi.json` shows no diff.

- [ ] **Step 4: Commit**

Mark TD-08 Done in the audit register, noting that the other files near the cap are split when a feature next touches them.

```bash
cargo clippy -p matrix-bot --all-targets -- -D warnings
git add backend/src/application/secrets docs/superpowers/reviews/2026-10-10-tech-debt-audit.md
git commit -m "refactor(secrets): split SecretService into reads, mutations and credential checks

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: TD-11 — job-level test for the price-refresh snapshot

**Files:**
- Modify: `backend/src/interface/jobs/portfolio.rs`, adding to `mod tests`.
- Modify: `docs/superpowers/HANDOFF.md` §4 Track 2. Remove the snapshot-test item.

**Interfaces:**
- Consumes: `RefreshPricesHandler::new(AppServices)`, the `services(&[&str])` and `ctx(Option<AccountId>)` test helpers already in that module, and `repositories::portfolio::history::get_snapshots(pool, &owner, "USD", &date)`.

- [ ] **Step 1: Write the test**

```rust
    /// The scheduled refresh records today's snapshot, and a retry of the
    /// same job does not record a second one.
    #[tokio::test]
    async fn refresh_prices_records_one_snapshot_per_day() {
        use crate::infrastructure::postgres::repositories::portfolio::history as snapshots;
        let (s, o) = services(&["acct_pf_h_snapshot"]).await;
        let handler = RefreshPricesHandler::new(s.clone());

        // No holdings, so no price is fetched and no network is touched.
        handler.run(&ctx(Some(o[0])), serde_json::json!({})).await.unwrap();
        handler.run(&ctx(Some(o[0])), serde_json::json!({})).await.unwrap();

        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let rows = snapshots::get_snapshots(s.portfolio.pool(), &o[0], "USD", &today)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "one snapshot per day, even after a retry");
    }

    #[tokio::test]
    async fn refresh_prices_without_an_account_is_permanent() {
        let (s, _) = services(&["acct_pf_h_noacct"]).await;
        let out = RefreshPricesHandler::new(s).run(&ctx(None), serde_json::json!({})).await;
        assert!(matches!(out, Err(JobError::Permanent(_))), "{out:?}");
    }
```

- [ ] **Step 2: Run them**

```bash
cargo test -p matrix-bot interface::jobs::portfolio::tests::refresh_prices
```

Expected: PASS, because the behaviour already exists and this adds missing coverage. To prove the first test can fail, temporarily comment out the `capture_current_snapshot_for_owner` call in `RefreshPricesHandler::run`, check that it FAILS with `left: 0`, and then restore the line.

If `JobError` does not implement `Debug`, replace `"{out:?}"` with a plain message.

- [ ] **Step 3: Commit**

Mark TD-11 Done in the audit register, and remove the item from HANDOFF Track 2.

```bash
git add backend/src/interface/jobs/portfolio.rs docs/superpowers/HANDOFF.md docs/superpowers/reviews/2026-10-10-tech-debt-audit.md
git commit -m "test(portfolio): the price-refresh job records one snapshot per day

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: TD-05 (admin module) — type the admin module and its fakes from the generated API

**Files:**
- Modify: `frontend/src/modules/admin/types.ts`, rewriting it as aliases of the generated types.
- Modify: the admin components or hooks that `tsc` flags after the switch.
- Create: `frontend/e2e/admin-fixtures.ts`, holding typed seed data and response builders.
- Modify: `frontend/e2e/fake-api.ts:415-479` and the `/admin/` handlers at about `:1209-1360`.
- Create: `frontend/tsconfig.e2e.json`.
- Modify: `frontend/package.json`, adding the `typecheck:e2e` script.
- Modify: `.github/workflows/ci.yml`, in the `frontend` job.

**Interfaces:**
- Produces, from `frontend/e2e/admin-fixtures.ts`:
  - `initialAdminSettings(): SettingItemDto[]`
  - `configStatus(settings: SettingItemDto[], secrets: SecretMetadataDto[]): ConfigurationStatusDto`
  - `auditPage(rows: ConfigAuditRowDto[], limit: number, offset: number): AuditLogPageDto`
  - the type aliases `FakeSetting`, `FakeSecret` and `FakeAuditRow`.

**Ruling:** Playwright transpiles but does not type-check, and `tsconfig.json` includes only `src`. So the typed shapes live in `admin-fixtures.ts`, which a dedicated `tsc -p tsconfig.e2e.json` checks in CI. Bringing all of `fake-api.ts` (3,700 lines) under `tsc` is Phase 2, done module by module.

- [ ] **Step 1: Make the admin types aliases of the generated ones**

Replace the whole of `frontend/src/modules/admin/types.ts` with:

```ts
import type { components } from '../../types/api';

type Schemas = components['schemas'];

// Aliases of the generated DTOs: a backend change that breaks the admin
// console now fails `npm run build`, not the browser.
export type SettingItem = Schemas['SettingItemDto'];
export type SecretMetadata = Schemas['SecretMetadataDto'];
export type SecretTestResult = Schemas['SecretTestResultDto'];
export type ConfigStatus = Schemas['ConfigurationStatusDto'];
export type AuditLogEntry = Schemas['ConfigAuditRowDto'];
export type AuditLogPage = Schemas['AuditLogPageDto'];
```

- [ ] **Step 2: Build, then fix each use site against the real shape**

```bash
npm --prefix frontend run build
```

Each error is a place where the UI assumed a field or a value set the backend does not have. That is the point of TD-05. Fix the component to match the generated type. Do not widen the type back to `any`. If a generated field is wrong, fix the backend DTO and regenerate: `cargo run --bin generate-openapi`, then `npm --prefix frontend run generate-api`.

Repeat until the build passes.

- [ ] **Step 3: Create the typed fixtures**

Create `frontend/e2e/admin-fixtures.ts`:

```ts
import type { components } from '../src/types/api';

type Schemas = components['schemas'];
export type FakeSetting = Schemas['SettingItemDto'];
export type FakeSecret = Schemas['SecretMetadataDto'];
export type FakeAuditRow = Schemas['ConfigAuditRowDto'];

/** The settings a fresh fake backend reports. A new array on every call. */
export function initialAdminSettings(): FakeSetting[] {
  return [
    // Move the five object literals from fake-api.ts `adminSettings` here
    // unchanged; `tsc -p tsconfig.e2e.json` now checks each field.
  ];
}

export function configStatus(
  settings: FakeSetting[],
  secrets: FakeSecret[],
): Schemas['ConfigurationStatusDto'] {
  return {
    // Move the object literal from the `/admin/config-status` handler here,
    // computing the counts from `settings` and `secrets` exactly as it does now.
  };
}

export function auditPage(
  rows: FakeAuditRow[],
  limit: number,
  offset: number,
): Schemas['AuditLogPageDto'] {
  return { entries: rows.slice(offset, offset + limit), total: rows.length, limit, offset };
}
```

The two "move here" comments mark a verbatim cut-and-paste from the lines named in **Files**. They are not code to invent. Delete each comment once its literal is in place.

- [ ] **Step 4: Use the fixtures in the fake API**

In `frontend/e2e/fake-api.ts`:
- Add `import { auditPage, configStatus, initialAdminSettings, type FakeAuditRow, type FakeSecret, type FakeSetting } from './admin-fixtures';`.
- Change the fields to `adminSettings: FakeSetting[] = initialAdminSettings();`, `adminSecrets: FakeSecret[] = [];` and `adminAuditLogs: FakeAuditRow[] = [];`.
- Make `/admin/config-status` return `json(route, 200, configStatus(this.adminSettings, this.adminSecrets))`.
- Make `/admin/audit-logs` return `json(route, 200, auditPage(this.adminAuditLogs, limit, offset))`.

- [ ] **Step 5: Type-check the fixtures, locally and in CI**

Create `frontend/tsconfig.e2e.json`:

```json
{
  "extends": "./tsconfig.json",
  "compilerOptions": { "noEmit": true },
  "include": ["e2e/admin-fixtures.ts", "src/types/api.ts"]
}
```

In `frontend/package.json` `scripts`, add `"typecheck:e2e": "tsc -p tsconfig.e2e.json"`.

In `.github/workflows/ci.yml`, in the `frontend` job, after `Frontend build`, add:

```yaml
      - name: Type-check typed e2e fixtures
        run: npm --prefix frontend run typecheck:e2e
```

Prove that the check bites. Temporarily rename `entries` to `items` in `auditPage`, run `npm --prefix frontend run typecheck:e2e`, and confirm it FAILS. Then revert the rename.

- [ ] **Step 6: Run everything**

```bash
npm --prefix frontend run build
npm --prefix frontend run typecheck:e2e
npm --prefix frontend run test:e2e
```

Expected: all green. `e2e/admin.spec.ts` passes against the typed fakes.

- [ ] **Step 7: Commit**

In the audit register, mark TD-05 **Partial**: the admin module is done, the other modules are Phase 2, and the rule is that new modules use generated types.

```bash
git add frontend .github/workflows/ci.yml docs/superpowers/reviews/2026-10-10-tech-debt-audit.md
git commit -m "test(admin): type the admin console and its e2e fakes from the generated API

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 10: Phase 1 close-out

**Files:**
- Modify: `docs/superpowers/HANDOFF.md`.
- Modify: `docs/superpowers/reviews/2026-10-10-tech-debt-audit.md`.

- [ ] **Step 1: Run the full AGENTS §8 gate set on the branch**

Run every command from Task 1, Step 5, plus:

```bash
cargo test -p matrix-bot-inference --all-targets
npm --prefix frontend run typecheck:e2e
git diff --check master...HEAD
```

Expected: everything green.

- [ ] **Step 2: Update the audit and HANDOFF**

1. **Audit:** add a line under "Phased remediation plan", Phase 1: `Status 2026-10-xx: done except TD-05 (admin only; remaining modules in Phase 2).`
2. **HANDOFF:**
   - Update the status table.
   - In Track 2, replace the audit pointer's plan link with "Phase 2 next: TD-10 (reqwest 0.12, own branch), TD-13".
   - Delete this plan file, since its work is done and AGENTS §10 says completed plans are deleted.

```bash
git rm docs/superpowers/plans/2026-10-11-tech-debt-phase-0-1.md
git add docs/superpowers/HANDOFF.md docs/superpowers/reviews/2026-10-10-tech-debt-audit.md
git commit -m "docs(handoff): close tech-debt phase 1

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 3: Hand over**

Report the gate results and the commit list. Ask the owner whether to push `chore/tech-debt-phase-1` to Gitea and open the PR. Do neither without their approval.
