-- Step 10: autonomous agent runtime and task framework.
-- Tables for scheduled agent tasks, execution runs, granular steps,
-- and human-in-the-loop approvals for sensitive tools.

CREATE TABLE scheduled_agent_tasks (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id      UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    task_id         TEXT NOT NULL,
    cron_expression TEXT NOT NULL,
    enabled         BOOLEAN NOT NULL DEFAULT TRUE,
    config          JSONB NOT NULL DEFAULT '{}'::jsonb,
    last_run_at     TIMESTAMPTZ,
    next_run_at     TIMESTAMPTZ NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_account_task UNIQUE (account_id, task_id)
);

CREATE INDEX idx_scheduled_agent_tasks_due 
    ON scheduled_agent_tasks (next_run_at) 
    WHERE enabled = TRUE;
CREATE INDEX idx_scheduled_agent_tasks_account 
    ON scheduled_agent_tasks (account_id);

CREATE TABLE agent_runs (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    scheduled_task_id   UUID REFERENCES scheduled_agent_tasks(id) ON DELETE SET NULL,
    account_id          UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    task_name           TEXT NOT NULL,
    goal                TEXT NOT NULL,
    origin_conversation TEXT,
    status              TEXT NOT NULL DEFAULT 'planning'
                        CHECK (status IN ('planning', 'executing', 'waiting_approval', 'completed', 'failed', 'cancelled')),
    max_steps           INTEGER NOT NULL DEFAULT 15,
    current_step        INTEGER NOT NULL DEFAULT 0,
    total_tokens        INTEGER NOT NULL DEFAULT 0,
    error               TEXT,
    started_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at        TIMESTAMPTZ,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_agent_runs_account 
    ON agent_runs (account_id, started_at DESC);
CREATE INDEX idx_agent_runs_active 
    ON agent_runs (account_id) 
    WHERE status IN ('planning', 'executing', 'waiting_approval');

CREATE TABLE agent_steps (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id          UUID NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    step_number     INTEGER NOT NULL,
    thought         TEXT,
    tool_name       TEXT,
    tool_input      JSONB,
    tool_output     JSONB,
    status          TEXT NOT NULL CHECK (status IN ('success', 'failed', 'blocked')),
    tokens_used     INTEGER NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_run_step UNIQUE (run_id, step_number)
);

CREATE INDEX idx_agent_steps_run 
    ON agent_steps (run_id, step_number ASC);

CREATE TABLE agent_approvals (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id          UUID NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    account_id      UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    tool_name       TEXT NOT NULL,
    tool_input      JSONB NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending'
                        CHECK (status IN ('pending', 'approved', 'rejected', 'expired')),
    expires_at      TIMESTAMPTZ NOT NULL,
    resolved_at     TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_agent_approvals_pending 
    ON agent_approvals (account_id, status) 
    WHERE status = 'pending';
CREATE INDEX idx_agent_approvals_run 
    ON agent_approvals (run_id);
