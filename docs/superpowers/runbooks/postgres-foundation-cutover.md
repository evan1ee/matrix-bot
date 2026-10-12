# PostgreSQL foundation cutover runbook

Durable version of the two pre-merge gates for `refactor/postgres-foundation`
(the workspace report files under `.superpowers/sdd/2026-09-24-postgres-only-and-migrations/`
are temporary). Follow this in order before merging, and again before the
first production boot on the new binary.

See `docs/superpowers/specs/2026-09-24-postgres-foundation-design.md` for the
design rationale and `docs/database.md` (Migrations),
`docs/architecture.md` (Matrix session and crypto store; Verification) for the invariants this
protects.

## 1. Before the first boot

1. Run the **pre-branch master** binary (master as it stood before
   `refactor/postgres-foundation` branched off) against production once, so
   its schema equals what `0001_baseline.sql` adopts. This is a
   precondition, not automatic — see
   `docs/database.md`'s Migrations section for why
   `0001` omits the backfill and rename it depends on already having run.
   On the old bot, also check the legacy webhook queue (migration `0005`
   drops `webhook_outbox`, undelivered rows included):
   `SELECT COUNT(*) FROM webhook_outbox WHERE status = 'pending'`. If it is
   non-zero, let the old bot drain it (re-run until 0) or accept losing
   those alerts before you continue.
2. **Stop the bot.**
3. Back up the whole `data/` directory — `bot_store/`, `matrix_session.json`,
   `matrix_token.json`, `verified_users.json`, and anything else it holds —
   and take a `pg_dump` of production immediately before the new binary's
   first boot. matrix-sdk-sqlite's store migrations are one-way: the 0.7
   binary cannot open a store that the 0.18 binary has migrated. This is the
   only way back if the new binary's first boot goes wrong; rollback (§5)
   restores `data/` and, if needed, the dump.
4. Set `.env`'s `DATABASE_URL` to the production PostgreSQL URL. The new
   binary refuses a `sqlite://` URL at boot (`Config::validate`).

## 2. Production rehearsal (before merge)

Rehearse the migrations against a restored copy of production **taken after
step 1.1 above** — the rehearsal dump must already have the pre-branch
master's schema, or it is rehearsing against a shape production was never
actually in. Never rehearse against production itself:

```bash
pg_dump -Fc "$PROD_URL" > prod.dump
createdb -h localhost -U postgres rehearsal
pg_restore -h localhost -U postgres -d rehearsal --no-owner prod.dump

MIGRATE_TEST_DATABASE_URL=postgres://postgres:bot@localhost:5432/rehearsal \
PG_TEST_URL=postgres://postgres:bot@localhost:5432/postgres \
  cargo test -p matrix-bot migrates_a_real_database -- --ignored --nocapture
```

If it reports drift, fix it with a new migration using the next free number
in `backend/migrations/` — **never edit `0001_baseline.sql`**, and never
assume the next number is `0002`: the design spec reserves that one for
`0002_account_uuid.sql`, so check `backend/migrations/` first and note that
inserting a migration ahead of the spec's later numbers shifts all of them.
Known candidate: `recurring_bills.next_due_at` default drift, if
production's column was added by the legacy `ALTER TABLE` path rather than
created fresh. The likely fix is
`ALTER TABLE recurring_bills ALTER COLUMN next_due_at DROP DEFAULT` (or
whatever matches `0001_baseline.sql`'s own `CREATE TABLE` for that column —
check it before writing the migration, don't assume).

Record the result (pass, or the migration that fixed the drift and why) here
or in the PR before merging. If step 1.1 is re-run after this rehearsal (e.g.
because more time has passed before cutover), re-run this rehearsal too —
its precondition no longer holds otherwise.

### 2a. Omnichannel migrations 0006-0008 (when `feat/omnichannel` ships with it)

The same restored copy must also take the omnichannel migrations, in the same
rehearsal run (0004-0008 apply in one boot). Two of them rewrite or drop data:

- **0007 backfills and drops `matrix_links`.** It starts with a guard that
  raises `matrix_links rows reference missing accounts: <mxids>` if any link
  points at an account that no longer exists; the message names the offending
  MXIDs. Check beforehand on the copy:
  ```sql
  SELECT l.mxid FROM matrix_links l LEFT JOIN accounts a ON a.id = l.account_id WHERE a.id IS NULL;
  ```
  A non-empty result stops the boot, so remove those rows (or restore the
  missing accounts) on production first; never edit `0007`. After a clean
  run `SELECT COUNT(*) FROM platform_identities WHERE platform = 'matrix'`
  must equal the old `matrix_links` count taken before the rehearsal.
- **0008 drops `matrix_link_challenges`.** Pending web-minted link codes and
  pending merge confirmations are not carried over.

Tell users before release: **`!link <code>` is gone.** Existing links keep
working (they were migrated), but a new link is started from the chat itself
with `!link` (Matrix) or `/link` (Discord, Telegram) in a direct message, which
replies with a 10-minute, single-use link to open in the browser. A link or
merge that was half finished in the web UI must be started again. Optionally
set `WEB_URL` (defaults to `WEBAUTHN_ORIGIN`) and the signup allowlists
(`DISCORD_ALLOWED_USERS`, `TELEGRAM_ALLOWED_USERS`, `EMAIL_ALLOWED_SENDERS`)
before the first boot.

### 2b. Storage and Autonomous Agent migrations 0009–0010

Migrations 0009 and 0010 apply automatically alongside the foundation:

- **0009 adds `receipt_storage_key` to `transactions`.** Additive column with
  index `idx_transactions_receipt_storage_key`. Non-locking and zero-downtime.
  Ensure storage configuration is present before booting if receipt image
  ingestion is enabled:
  - `STORAGE_DRIVER`: `local` (default) or `s3`; `STORAGE_SIGNING_SECRET` signs presigned URLs.
  - For local storage: `STORAGE_LOCAL_DIR` (defaults to `data/storage`).
  - For S3: `S3_BUCKET`, `S3_REGION`, `S3_ACCESS_KEY`, `S3_SECRET_KEY`, and optional `S3_ENDPOINT` and `S3_FORCE_PATH_STYLE`.
- **0010 creates the Autonomous Agent subsystem.** Adds `agent_runs`,
  `agent_steps`, `agent_approvals`, and `scheduled_agent_tasks`. All tables are
  owner-keyed (`owner_id UUID REFERENCES accounts(id) ON DELETE CASCADE`) and
  indexed for fast lookup by owner and status.
  - The job queue registers handler `agent.execute_run`.
  - The scheduler registers recurring task tick `scheduler.agent_tasks` (every 60s).

### 2c. Multi-channel environment checklist

Before starting the bot, verify all channel credentials in `.env`:

| Channel | Key Variables | Pre-flight Notes |
|---|---|---|
| **Matrix** | `MATRIX_HOMESERVER`, `MATRIX_USER`, `MATRIX_DEVICE_ID` | Session and crypto store in `data/bot_store/` |
| **Discord** | `DISCORD_TOKEN`, `DISCORD_ALLOWED_USERS` | Bot requires Gateway Message Content intent for DM handling |
| **Telegram** | `TELEGRAM_BOT_TOKEN`, `TELEGRAM_MODE`, `TELEGRAM_ALLOWED_USERS` | Webhook mode requires `TELEGRAM_WEBHOOK_SECRET` and HTTPS `WEB_URL` |
| **Email** | `EMAIL_IMAP_URL`, `EMAIL_SMTP_URL`, `EMAIL_USERNAME`, `EMAIL_PASSWORD`, `EMAIL_FROM`, `EMAIL_AUTHSERV_ID` | Requires DMARC pass validation against `EMAIL_AUTHSERV_ID` |
| **Storage** | `STORAGE_DRIVER`, `STORAGE_LOCAL_DIR`, `STORAGE_SIGNING_SECRET`, `S3_BUCKET` | Directory must be writable by bot process |
| **LLM** | `AI_ENDPOINT`, `AI_API_KEY`, `AI_MODEL` | Used for interactive turn tools and agent tool-calling loop |

## 3. First boot on matrix-sdk 0.18 and PostgreSQL 18

Watch the log for, in order:

- **Store migration.** matrix-sdk-sqlite schema-upgrade messages for both the
  state store and the crypto store under `data/bot_store/`. The absence of
  any "failed to open/migrate store" error matters more than the presence of
  a success message.
- **Restored session, not a fresh login.** The restored-session branch of
  `interface/channels/matrix/client.rs` must run (`restore_session` then a successful `whoami`).
  Stop the process immediately if either of these appears instead:
  - a fresh-login path being taken
  - a `data/bot_store.orphaned-<timestamp>/` archive being created — this
    means the restore failed and a new device was minted, which peers will
    see as broken encryption (see `docs/architecture.md`'s Matrix session and
    crypto store section)
- **`ensure_self_verified` result.** Must log that the device signature was
  uploaded (`Device::verify()` returning `Ok`). A "nothing to sign" result or
  a timeout is a failure — never treat `Recovery::recover()`'s own `Ok` as
  "verified" (see `docs/architecture.md`'s Verification section).
- **Sync running.** The sync loop starts with no decryption errors on the
  first messages in encrypted rooms.
- **Chat identities.** After the omnichannel migrations,
  `SELECT platform, COUNT(*) FROM platform_identities GROUP BY platform` shows
  every previously linked Matrix account, `to_regclass('matrix_links')` is
  null, and `!link` in a DM answers with a `/link?token=` URL (never post
  that URL anywhere shared).
- **Migration bookkeeping.** `_sqlx_migrations` has a row for version 1 with
  `success = t`:
  ```sql
  SELECT version, success FROM _sqlx_migrations ORDER BY version;
  ```
- **Session reuse log line.** The log shows `Reusing stored Matrix session for
  ... on device <same ID as before>` (`interface/channels/matrix/client.rs`'s restore branch) — the
  same check as "Restored session, not a fresh login" above, called out
  explicitly because it's the one line to grep for.
- **New matrix-sdk store files.** `data/bot_store/matrix-sdk-event-cache.sqlite3`
  and `data/bot_store/matrix-sdk-media.sqlite3` should appear alongside the
  existing store files. Both are expected on 0.18 and are not a sign of a
  failed restore.
- **`!say` renders as a voice bubble.** Send `!say` and confirm the client
  (e.g. Element) shows a voice message with waveform bars, not a plain audio
  attachment — confirms the Ogg Opus reply is still being produced correctly
  after the upgrade.

If any of these fail, stop the bot and go to Rollback (§5) rather than
retrying boot.

## 4. After a successful boot

`docs/architecture.md` currently carries a placeholder note next to "Two
measured facts about matrix-sdk 0.7" (in the Verification section) because
those two facts — `Recovery::recover` returning `Ok` with nothing signed, and
`Device::is_cross_signed_by_owner()` lagging a re-query — were measured
against matrix-sdk 0.7 and have not been reconfirmed against 0.18. Re-measure
both against this boot's logs and update that paragraph (or confirm it still
holds and remove the placeholder note).

## 5. Rollback

The database normally needs no rollback: `0001_baseline.sql` is a no-op
against the already-adopted schema, so the previous binary (which never
applied `sqlx` migrations) sees no difference. If a later migration did run
and needs undoing, restore the `pg_dump` taken in step 1.3 instead of trying
to hand-write a down migration. This applies in particular to 0007 and 0008,
which drop `matrix_links` and `matrix_link_challenges`: the previous binary
cannot run against a database that has lost them, so a rollback past 0007 is a
dump restore, never a code-only revert.

To roll back the Matrix side:

1. Stop the new binary.
2. Restore the `data/` backup from step 1.3 (`bot_store/`,
   `matrix_session.json`, `matrix_token.json`, `verified_users.json`, …).
3. Run the previous (pre-branch) binary against the restored store.
