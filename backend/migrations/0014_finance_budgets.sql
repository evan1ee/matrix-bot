CREATE TABLE finance_budgets (
    id                BIGSERIAL PRIMARY KEY,
    owner_id          UUID    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    category_id       BIGINT  REFERENCES finance_categories(id) ON DELETE CASCADE,
    currency          TEXT    NOT NULL,
    currency_exponent BIGINT  NOT NULL,
    limit_minor       BIGINT  NOT NULL CHECK (limit_minor > 0),
    alerts_enabled    BOOLEAN NOT NULL DEFAULT TRUE,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- One budget per scope and currency; NULL (overall) is a value here.
CREATE UNIQUE INDEX idx_finance_budgets_scope
    ON finance_budgets (owner_id, category_id, currency) NULLS NOT DISTINCT;

CREATE TABLE finance_budget_alerts (
    budget_id   BIGINT   NOT NULL REFERENCES finance_budgets(id) ON DELETE CASCADE,
    month_start DATE     NOT NULL,          -- first day of the month in the owner's zone
    threshold   SMALLINT NOT NULL CHECK (threshold IN (80, 100)),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (budget_id, month_start, threshold)
);
