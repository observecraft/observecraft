-- Cloud Cost configuration, kept as Datadog's Cloud Cost Management API and Terraform provider
-- manage it (routes/datadog_api/cost.rs, routes/cloud_cost.rs).

-- Budgets (`datadog_cost_budget`): a cost query and a monthly amount per month and tag combination.
CREATE TABLE cost_budgets (
    org_id        TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    id            UUID NOT NULL DEFAULT gen_random_uuid(),
    name          TEXT NOT NULL,
    metrics_query TEXT NOT NULL,
    start_month   INTEGER NOT NULL,
    end_month     INTEGER NOT NULL,
    -- [{"month": 202601, "amount": 1000, "tag_filters": [{"tag_key": "team", "tag_value": "billing"}]}]
    entries       JSONB NOT NULL DEFAULT '[]',
    created_by    UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_by    UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, id)
);

-- A budget's custom forecast (`datadog_cost_custom_forecast`): overrides of the forecast per month and
-- tag combination of its entries.
CREATE TABLE cost_custom_forecasts (
    org_id     TEXT NOT NULL,
    budget_id  UUID NOT NULL,
    id         UUID NOT NULL DEFAULT gen_random_uuid(),
    entries    JSONB NOT NULL DEFAULT '[]',
    created_by UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, budget_id),
    FOREIGN KEY (org_id, budget_id) REFERENCES cost_budgets(org_id, id) ON DELETE CASCADE
);

-- Custom costs files (`PUT /api/v2/cost/custom_costs`): their line items are in ClickHouse
-- (otel.cloud_costs, FileId = id).
CREATE TABLE cost_custom_files (
    org_id         TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    id             UUID NOT NULL DEFAULT gen_random_uuid(),
    name           TEXT NOT NULL,
    provider_names TEXT[] NOT NULL DEFAULT '{}',
    billed_cost    DOUBLE PRECISION NOT NULL DEFAULT 0,
    currency       TEXT NOT NULL DEFAULT 'USD',
    -- Epoch milliseconds of the earliest ChargePeriodStart and latest ChargePeriodEnd.
    period_start   BIGINT NOT NULL,
    period_end     BIGINT NOT NULL,
    line_items     INTEGER NOT NULL,
    status         TEXT NOT NULL DEFAULT 'active',
    uploaded_by    UUID REFERENCES users(id) ON DELETE SET NULL,
    uploaded_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, id)
);
