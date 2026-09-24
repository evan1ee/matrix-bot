# PostgreSQL foundation cutover runbook

Durable version of the two pre-merge gates for `refactor/postgres-foundation`
(the workspace report files under `.superpowers/sdd/2026-09-24-postgres-only-and-migrations/`
are temporary). Follow this in order before merging, and again before the
first production boot on the new binary.

See `docs/superpowers/specs/2026-09-24-postgres-foundation-design.md` for the
design rationale and `docs/ARCHITECTURE.md` (Persistence and migrations;
Matrix session and crypto store; Verification) for the invariants this
protects.

## 1. Production rehearsal (before merge)

Rehearse the migrations against a restored copy of production, never against
production itself:

```bash
pg_dump -Fc "$PROD_URL" > prod.dump
createdb -h localhost -U postgres rehearsal
pg_restore -h localhost -U postgres -d rehearsal --no-owner prod.dump

MIGRATE_TEST_DATABASE_URL=postgres://postgres:bot@localhost:5432/rehearsal \
PG_TEST_URL=postgres://postgres:bot@localhost:5432/postgres \
  cargo test -p matrix-bot migrates_a_real_database -- --ignored --nocapture
```

If it reports drift, fix it with a new `0002_*` migration — **never edit
`0001_baseline.sql`**. Known candidate: `recurring_bills.next_due_at` default
drift, if production's column was added by the legacy `ALTER TABLE` path
rather than created fresh. The likely fix is
`ALTER TABLE recurring_bills ALTER COLUMN next_due_at DROP DEFAULT` (or
whatever matches `0001_baseline.sql`'s own `CREATE TABLE` for that column —
check it before writing the migration, don't assume).

Record the result (pass, or the migration that fixed the drift and why) here
or in the PR before merging.

## 2. Before the first boot

1. Run the **pre-branch master** binary (master as it stood before
   `refactor/postgres-foundation` branched off) against production once, so
   its schema equals what `0001_baseline.sql` adopts. This is a
   precondition, not automatic — see
   `docs/ARCHITECTURE.md`'s Persistence and migrations section for why
   `0001` omits the backfill and rename it depends on already having run.
2. **Stop the bot.**
3. Back up `data/bot_store/` and the session file
   (`data/matrix_session.json`). matrix-sdk-sqlite's store migrations are
   one-way: the 0.7 binary cannot open a store that the 0.18 binary has
   migrated. This backup is the only way back if the new binary's first boot
   goes wrong.
4. Set `.env`'s `DATABASE_URL` to the production PostgreSQL URL. The new
   binary refuses a `sqlite://` URL at boot (`Config::validate`).

## 3. First boot on matrix-sdk 0.18

Watch the log for, in order:

- **Store migration.** matrix-sdk-sqlite schema-upgrade messages for both the
  state store and the crypto store under `data/bot_store/`. The absence of
  any "failed to open/migrate store" error matters more than the presence of
  a success message.
- **Restored session, not a fresh login.** The restored-session branch of
  `bot/client.rs` must run (`restore_session` then a successful `whoami`).
  Stop the process immediately if either of these appears instead:
  - a fresh-login path being taken
  - a `data/bot_store.orphaned-<timestamp>/` archive being created — this
    means the restore failed and a new device was minted, which peers will
    see as broken encryption (see `docs/ARCHITECTURE.md`'s Matrix session and
    crypto store section)
- **`ensure_self_verified` result.** Must log that the device signature was
  uploaded (`Device::verify()` returning `Ok`). A "nothing to sign" result or
  a timeout is a failure — never treat `Recovery::recover()`'s own `Ok` as
  "verified" (see `docs/ARCHITECTURE.md`'s Verification section).
- **Sync running.** The sync loop starts with no decryption errors on the
  first messages in encrypted rooms.
- **Migration bookkeeping.** `_sqlx_migrations` has a row for version 1 with
  `success = t`:
  ```sql
  SELECT version, success FROM _sqlx_migrations ORDER BY version;
  ```

If any of these fail, stop the bot and go to Rollback (§5) rather than
retrying boot.

## 4. After a successful boot

`docs/ARCHITECTURE.md` currently carries a placeholder note next to "Two
measured facts about matrix-sdk 0.7" (in the Verification section) because
those two facts — `Recovery::recover` returning `Ok` with nothing signed, and
`Device::is_cross_signed_by_owner()` lagging a re-query — were measured
against matrix-sdk 0.7 and have not been reconfirmed against 0.18. Re-measure
both against this boot's logs and update that paragraph (or confirm it still
holds and remove the placeholder note).

## 5. Rollback

The database needs no rollback: `0001_baseline.sql` is a no-op against the
already-adopted schema, so the previous binary (which never applied `sqlx`
migrations) sees no difference.

To roll back the Matrix side:

1. Stop the new binary.
2. Restore the `data/bot_store/` and session-file backup from step 2.3.
3. Run the previous (pre-branch) binary against the restored store.
