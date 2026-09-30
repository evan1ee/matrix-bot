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

## 3. First boot on matrix-sdk 0.18

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
to hand-write a down migration.

To roll back the Matrix side:

1. Stop the new binary.
2. Restore the `data/` backup from step 1.3 (`bot_store/`,
   `matrix_session.json`, `matrix_token.json`, `verified_users.json`, …).
3. Run the previous (pre-branch) binary against the restored store.
