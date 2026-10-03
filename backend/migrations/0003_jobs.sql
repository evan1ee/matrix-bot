-- 0003_jobs.sql: durable job queue (design step 8).
-- Spec: docs/superpowers/specs/2026-09-29-job-queue-design.md §3
-- Plain DDL; sqlx runs this file in one transaction.

CREATE TABLE jobs (
    id              BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    account_id      UUID REFERENCES accounts(id) ON DELETE CASCADE,
    job_type        TEXT NOT NULL,
    payload         JSONB NOT NULL DEFAULT '{}'::jsonb,
    status          TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending','running','completed','failed','cancelled')),
    priority        SMALLINT NOT NULL DEFAULT 0,
    attempts        INTEGER NOT NULL DEFAULT 0,
    max_attempts    INTEGER NOT NULL DEFAULT 3 CHECK (max_attempts >= 1),
    backoff_seconds INTEGER NOT NULL DEFAULT 10 CHECK (backoff_seconds >= 0),
    available_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    locked_at       TIMESTAMPTZ,
    locked_by       TEXT,
    started_at      TIMESTAMPTZ,
    completed_at    TIMESTAMPTZ,
    heartbeat_at    TIMESTAMPTZ,
    dedupe_key      TEXT,
    error           TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_jobs_dedupe UNIQUE (job_type, dedupe_key)
);

CREATE INDEX idx_jobs_dequeue ON jobs (priority DESC, available_at ASC) WHERE status = 'pending';
CREATE INDEX idx_jobs_running ON jobs (heartbeat_at) WHERE status = 'running';
CREATE INDEX idx_jobs_finished ON jobs (completed_at) WHERE status IN ('completed','failed','cancelled');
CREATE INDEX idx_jobs_account ON jobs (account_id);

-- Wakes idle workers (LISTEN jobs). A row trigger fires only for rows really
-- inserted, so an ON CONFLICT DO NOTHING skip sends nothing; NOTIFY is
-- delivered at commit, so a worker never sees an uncommitted job.
CREATE FUNCTION jobs_notify() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    PERFORM pg_notify('jobs', '');
    RETURN NULL;
END
$$;

CREATE TRIGGER jobs_notify AFTER INSERT ON jobs
    FOR EACH ROW EXECUTE FUNCTION jobs_notify();
