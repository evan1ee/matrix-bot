# Development Environment & Testing

How to set up a machine to build and test `matrix-bot`, and how to run every test suite the way CI does. Written for people and for coding agents taking over work (see also `docs/HANDOFF.md`, `docs/GITEA.md`, `AGENTS.md`).

> **Agents:** you never need Matrix credentials, an AI key or a production database to build and test. Everything below runs against a throwaway local PostgreSQL. **Do not run the bot binary** (`cargo run` / `matrix-bot`) — see §6.

---

## 1. Toolchain

| Tool | Version | Why |
|---|---|---|
| Rust | stable (edition 2021), with `rustfmt` and `clippy` | backend + inference crates; CI uses `dtolnay/rust-toolchain@stable` |
| Docker | any recent (Docker Desktop on Windows) | local PostgreSQL 18 for tests |
| PostgreSQL | **18** (via Docker image `postgres:18`) | the only application database |
| `sqlx-cli` | **0.9**, postgres + rustls only | regenerate / check the `.sqlx/` query cache, run migrations |
| Node.js | **20** (CI) — 20+ locally | frontend build, OpenAPI type generation, Playwright |
| Git | any | remotes: see `docs/GITEA.md` |

Install `sqlx-cli` exactly as CI does:

```bash
cargo install sqlx-cli --version ^0.9 --no-default-features --features rustls,postgres --locked
```

### OS-specific

- **Linux / CI containers:** `apt-get install -y pkg-config libssl-dev` (reqwest's native-tls). The inference crate also needs `cmake`.
- **Windows:**
  - A cold build of `openssl-sys` needs a **full Perl** on `PATH` (Strawberry Perl or `scoop install perl`). Git for Windows' bundled Perl lacks `Locale::Maketext::Simple` and fails.
  - Loading the real Kokoro TTS model in an inference test needs sherpa's own `onnxruntime.dll` copied into `target/debug/deps/` (see the comment in `inference/src/kokoro.rs`); ordinary test runs don't.
  - Line endings: `.gitattributes` pins `backend/migrations/*.sql` and `.sqlx/` to **LF**. Don't override it — a migration's checksum is computed over its bytes, and a CRLF checkout would fail `sqlx migrate` against an existing database.
- **Faster rebuilds across worktrees:** point several checkouts at one target dir, e.g. `export CARGO_TARGET_DIR=/path/to/shared/target`.

## 2. Repository at a glance

```
backend/      crate `matrix-bot` (lib + bins `matrix-bot`, `generate-openapi`); migrations in backend/migrations/
inference/    crate `matrix-bot-inference` (local ONNX OCR / STT / TTS server; heavy native deps)
frontend/     React 19 + Vite SPA → builds into backend/web/dist
contracts/    JSON wire fixtures asserted by both crates
.sqlx/        committed compile-time query cache (SQLX_OFFLINE=true via .cargo/config.toml)
```

`default-members = ["backend"]`: bare `cargo build`/`cargo test` only touch the bot. Always scope explicitly (`-p matrix-bot`, `-p matrix-bot-inference`).

## 3. Local PostgreSQL for tests

```bash
docker run -d --name pg18-test --shm-size=1g -p 55433:5432 \
  -e POSTGRES_PASSWORD=bot postgres:18
```

Environment variables the tests read:

| Variable | Used by | Value (local) |
|---|---|---|
| `TEST_DATABASE_URL` | every DB test (`db::pool::test_pool`) | `postgres://postgres:bot@localhost:55433/postgres` |
| `PG_TEST_URL` | the `#[ignore]`d `live_postgres` tests | same URL |
| `DATABASE_URL` | `sqlx migrate run` / `cargo sqlx prepare` only | same URL |
| `MIGRATE_TEST_DATABASE_URL` | the production rehearsal test only (§5.6) | a **restored copy** of production — never production itself |

```bash
export TEST_DATABASE_URL=postgres://postgres:bot@localhost:55433/postgres
export PG_TEST_URL=$TEST_DATABASE_URL
```

(PowerShell: `$env:TEST_DATABASE_URL = "postgres://postgres:bot@localhost:55433/postgres"`.)

**Per-test schemas.** `test_pool()` creates a fresh schema per test (`t<pid>_<n>`) on that server and returns an **unmigrated** pool; tests call `run_migrations_unlocked` themselves. Schemas are deliberately not dropped (a failing test keeps its rows for inspection), so they accumulate: that is why the container needs `--shm-size=1g`. Clean up between long sessions:

```bash
docker rm -f pg18-test   # then re-run the docker run above
```

Port **55433** avoids clashing with any PostgreSQL already on 5432. CI uses the service label instead (`postgres://postgres:bot@postgres:5432/postgres`) because its jobs run inside containers (see `docs/GITEA.md` §3).

## 4. Configuration (`.env`)

- The bot reads `.env` **only from the current working directory** (`main.rs` → `load_dotenv_from`), never a parent directory. The architecture test forbids `dotenvy::dotenv`.
- Start from `.env.example`. `DATABASE_URL` must be a `postgres://` URL (no default, no SQLite). `ENVIRONMENT=development` relaxes production checks (`COOKIE_SECURE=false`, localhost WebAuthn).
- **Tests and builds need no `.env`.** `SQLX_OFFLINE=true` (in `.cargo/config.toml`) makes `query!` read `.sqlx/`, never a live database at build time.

## 5. Running the tests

Run from the repository root. This is the full list a change must pass (it mirrors `.github/workflows/ci.yml`); `AGENTS.md` → *Definition of Done* is the authority.

### 5.1 Backend (every change)

```bash
cargo fmt --all -- --check                               # CI blocks on this; `cargo fmt --all` fixes
cargo check -p matrix-bot --all-targets                  # must be warning-free
cargo test  -p matrix-bot --all-targets                  # full suite (needs TEST_DATABASE_URL)
cargo test  -p matrix-bot --test architecture            # layer/dependency rules (no DB needed)
cargo test  -p matrix-bot live_postgres -- --ignored --test-threads=1   # needs PG_TEST_URL
cargo clippy --workspace --all-targets -- -D warnings    # must be clean
```

Narrow runs while iterating:

```bash
cargo test -p matrix-bot settings::                      # one module path
cargo test -p matrix-bot test_parse_message_exclamation_command   # one test by name
cargo test -p matrix-bot -- --nocapture                  # see println!/tracing output
```

### 5.2 Query cache (whenever SQL or a migration changes)

```bash
export DATABASE_URL=$TEST_DATABASE_URL
sqlx migrate run --source backend/migrations             # bring the scratch DB to the latest schema
cargo sqlx prepare --workspace -- -p matrix-bot --all-targets          # regenerate .sqlx/ → commit it
cargo sqlx prepare --workspace --check -- -p matrix-bot --all-targets  # what CI runs: must pass
```

A build error like "`SQLX_OFFLINE=true` but there is no cached data for this query" means `.sqlx/` is stale — regenerate it as above.

**Migrations** live in `backend/migrations/NNNN_name.sql`, forward-only. Never edit an applied migration (its checksum is recorded in `_sqlx_migrations`); add a new one.

### 5.3 OpenAPI contract (every change)

```bash
cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json
npm --prefix frontend run generate-api                   # only if openapi.json legitimately changed
```

During the current refactor the HTTP contract is frozen: **any** diff in `openapi.json` is a failure (see `docs/HANDOFF.md`). `generate-openapi` is the only binary agents may run.

### 5.4 Inference crate (when `inference/` or `contracts/` changes)

```bash
cargo fmt --all -- --check
cargo test -p matrix-bot-inference --all-targets
```

It links `sherpa-onnx` (shared) and downloads its prebuilt archive at build time; the first build is slow and needs network access to GitHub releases. Tests that load real models are `#[ignore]`d.

### 5.5 Frontend (when `frontend/` or `openapi.json` changes)

```bash
npm --prefix frontend ci
npm --prefix frontend run build          # tsc + vite build → backend/web/dist  (CI gate)
npx --prefix frontend playwright install chromium   # once per machine
npm --prefix frontend run test:e2e       # Playwright, required before shipping a UI change
```

The Playwright suite starts the Vite dev server itself (`localhost:5173`) and intercepts the REST API in `frontend/e2e/fake-api.ts` — **no backend, database or Matrix needed**. Add mock routes there for new endpoints. Run through `npm --prefix frontend run …` (a bare `npx --prefix … playwright` skips the config).

### 5.6 Special tests (user-supervised)

- `migrates_a_real_database` — migrates a **copy** of production and asserts nothing is stranded:
  `MIGRATE_TEST_DATABASE_URL=postgres://…/rehearsal cargo test -p matrix-bot migrates_a_real_database -- --ignored`.
  Needs a `pg_dump` of production restored into a scratch DB; follow `docs/runbooks/postgres-foundation-cutover.md`. The user runs or approves this.
- `self_verify` (`cargo test -p matrix-bot self_verify -- --ignored`) contacts the live homeserver — **agents never run it**.

## 6. What agents must not do

- Don't run `cargo run` / the `matrix-bot` binary, `docker compose up` of the bot, or any test that talks to a homeserver, LLM provider or search API. The bot logs in to Matrix on boot; a second login on the bot's device can consume one-time keys and break encryption.
- Don't read or modify the user's main-checkout `.env` (production credentials) or `data/` (the real Matrix crypto store). Work in a separate worktree/clone.
- Don't point `TEST_DATABASE_URL`, `PG_TEST_URL` or `DATABASE_URL` at anything but a throwaway local server.

## 7. Running the app locally (humans only)

For a human developer with their own test Matrix account:

```bash
cp .env.example .env            # fill in MATRIX_*, DATABASE_URL (a dev database), AI_* as needed
cargo run                       # bot + API on :8080 (run from the repo root)
npm --prefix frontend run dev   # SPA on :5173, proxies /api and /health to :8080
```

Or with Docker Compose (bot + PostgreSQL):

```bash
DATABASE_URL=postgres://bot:bot@postgres:5432/matrix_bot ENVIRONMENT=development \
WEBAUTHN_ORIGIN=http://localhost:8080 docker compose --profile postgres up -d
```

Probes: `GET /health/live` (process up) and `GET /health/ready` (database reachable within 2 s and a Matrix sync within the last 120 s). Passkey sign-in against localhost: see `docs/testing-auth-locally.md`.

## 8. Troubleshooting

| Symptom | Fix |
|---|---|
| `TEST_DATABASE_URL must point at a PostgreSQL server` | export `TEST_DATABASE_URL` (§3) and check `docker ps` shows `pg18-test` |
| tests fail with `could not resize shared memory segment` / out of shared memory | too many leftover test schemas — recreate the container with `--shm-size=1g` (§3) |
| `no cached data for this query` at compile time | `.sqlx/` stale — §5.2 |
| `migration … was previously applied but has been modified` | a migration file changed (often CRLF) — restore it; line endings are pinned to LF |
| `openssl-sys` build fails on Windows | install a full Perl (§1) |
| `git diff --exit-code backend/openapi.json` fails | an HTTP type changed; during the refactor that is a bug — revert the schema change |
| architecture test fails | a layer imports what it must not, or a file left `INTERFACE_SQL_ALLOW_LIST` without being fixed; read the message — it names the file |
| Playwright can't find browsers | `npx --prefix frontend playwright install chromium` |
