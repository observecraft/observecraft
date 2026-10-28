-- Developer Portal → Scorecards: rules evaluated per service, grouped into scorecards, with levels
-- (1 basic … 3 advanced). Built-in rules read the org's telemetry, monitors, SLOs and the service
-- metadata below; custom rules get their outcomes from users or the API. See routes/scorecards.rs.

-- What a service's owners declare about it: team, contacts, code repository and docs.
CREATE TABLE service_metadata (
    org_id     TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    service    TEXT NOT NULL CHECK (length(service) BETWEEN 1 AND 200),
    -- Team handle (`checkout`), or '' when no team owns the service.
    team       TEXT NOT NULL DEFAULT '',
    -- [{"type": "email" | "slack" | "link", "name": "…", "value": "…"}, …]
    contacts   JSONB NOT NULL DEFAULT '[]',
    repo_url   TEXT NOT NULL DEFAULT '',
    docs_url   TEXT NOT NULL DEFAULT '',
    updated_by UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, service)
);

-- One row per org once its scorecards are set up; schedules the periodic evaluation. Evaluators
-- claim due orgs with FOR UPDATE SKIP LOCKED and hold `claimed_until` while they run.
CREATE TABLE scorecard_settings (
    org_id        TEXT PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    next_eval_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    claimed_until TIMESTAMPTZ,
    evaluated_at  TIMESTAMPTZ,
    -- Telemetry facts per service from the last full evaluation (versions on spans, logs with trace
    -- ids, CI deployments), reused when a rule or service metadata changes in between.
    telemetry     JSONB NOT NULL DEFAULT '{}',
    telemetry_at  TIMESTAMPTZ,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX scorecard_settings_due ON scorecard_settings (next_eval_at);

CREATE TABLE scorecards (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name        TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    description TEXT NOT NULL DEFAULT '',
    position    INT NOT NULL DEFAULT 0,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, name),
    UNIQUE (org_id, id)
);

CREATE TABLE scorecard_rules (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id       TEXT NOT NULL,
    scorecard_id UUID NOT NULL,
    -- Key of a built-in rule (`monitors_defined`, …); NULL for a custom rule.
    builtin      TEXT,
    name         TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    description  TEXT NOT NULL DEFAULT '',
    level        SMALLINT NOT NULL CHECK (level BETWEEN 1 AND 3),
    -- Which services the rule applies to: a service search (`service:order-* team:checkout`); '' for all.
    scope        TEXT NOT NULL DEFAULT '',
    enabled      BOOLEAN NOT NULL DEFAULT true,
    created_by   UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, builtin),
    UNIQUE (org_id, id),
    FOREIGN KEY (org_id, scorecard_id) REFERENCES scorecards(org_id, id) ON DELETE CASCADE
);
CREATE INDEX scorecard_rules_org ON scorecard_rules (org_id);

-- Outcomes of custom rules, reported per service by users or the API.
CREATE TABLE scorecard_manual_outcomes (
    org_id     TEXT NOT NULL,
    rule_id    UUID NOT NULL,
    service    TEXT NOT NULL,
    state      TEXT NOT NULL CHECK (state IN ('pass', 'fail', 'skip')),
    remarks    TEXT NOT NULL DEFAULT '',
    updated_by UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (rule_id, service),
    FOREIGN KEY (org_id, rule_id) REFERENCES scorecard_rules(org_id, id) ON DELETE CASCADE
);
CREATE INDEX scorecard_manual_outcomes_org ON scorecard_manual_outcomes (org_id);

-- The last evaluation: one row per (enabled rule, service in its scope), replaced as a whole.
CREATE TABLE scorecard_outcomes (
    org_id       TEXT NOT NULL,
    rule_id      UUID NOT NULL,
    service      TEXT NOT NULL,
    state        TEXT NOT NULL CHECK (state IN ('pass', 'fail', 'skip')),
    reason       TEXT NOT NULL DEFAULT '',
    evaluated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (rule_id, service),
    FOREIGN KEY (org_id, rule_id) REFERENCES scorecard_rules(org_id, id) ON DELETE CASCADE
);
CREATE INDEX scorecard_outcomes_org ON scorecard_outcomes (org_id);
