-- Software Delivery → Deployment Gates and PR Gates. Managed through routes/deployment_gates.rs
-- and routes/pr_gates.rs; CI pipelines call the evaluation endpoints with an application key.

-- A deployment gate: the rules a deployment of `service` to `env` must pass. `identifier` tells
-- several gates of one service and environment apart (`default`, `pre-deploy`, `canary`, …).
CREATE TABLE deployment_gates (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    service     TEXT NOT NULL,
    env         TEXT NOT NULL,
    identifier  TEXT NOT NULL DEFAULT 'default',
    -- A gate in dry run always lets the deployment through; evaluations still record what failed.
    dry_run     BOOLEAN NOT NULL DEFAULT false,
    -- [{"type": "monitor", "name": …, "query": "service:billing env:prod", "dryRun": false},
    --  {"type": "faulty_deployment", "name": …, "windowMinutes": 15, "maxErrorRateIncrease": 1.0, "minRequests": 50, "dryRun": false}]
    rules       JSONB NOT NULL DEFAULT '[]',
    created_by  TEXT NOT NULL DEFAULT '',
    updated_by  TEXT NOT NULL DEFAULT '',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX deployment_gates_scope ON deployment_gates (org_id, service, env, identifier);

-- One evaluation of a gate, requested by a CI/CD pipeline before (or during) a deployment.
CREATE TABLE deployment_gate_evaluations (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id       TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    gate_id      UUID REFERENCES deployment_gates(id) ON DELETE SET NULL,
    service      TEXT NOT NULL,
    env          TEXT NOT NULL,
    identifier   TEXT NOT NULL,
    -- The version being deployed (`service.version` of its spans); '' when the caller didn't send one.
    version      TEXT NOT NULL DEFAULT '',
    -- passed | failed | dry_run (would have failed, but only dry-run rules or a dry-run gate did).
    status       TEXT NOT NULL CHECK (status IN ('passed', 'failed', 'dry_run')),
    -- Each rule's outcome at evaluation time, monitors in alert included.
    rule_results JSONB NOT NULL DEFAULT '[]',
    requested_by TEXT NOT NULL DEFAULT '',
    started_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    duration_ms  INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX deployment_gate_evaluations_org ON deployment_gate_evaluations (org_id, started_at DESC);
CREATE INDEX deployment_gate_evaluations_gate ON deployment_gate_evaluations (org_id, gate_id, started_at DESC);

-- PR Gates: rules a pull request's commit must pass, from CI Visibility data (pipelines, tests) or,
-- for the Code Security sources, kept until that product exists (they aren't evaluated yet).
CREATE TABLE pr_gate_rules (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id       TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    source       TEXT NOT NULL CHECK (source IN ('pipeline_failures', 'test_failures', 'flaky_tests',
                                                 'code_vulnerabilities', 'code_quality', 'library_vulnerabilities', 'license_violations')),
    -- Repositories the rule applies to (`example-org/billing`); empty = every repository.
    repositories TEXT[] NOT NULL DEFAULT '{}',
    -- The rule fails when the count of findings is above this.
    threshold    INTEGER NOT NULL DEFAULT 0 CHECK (threshold >= 0),
    -- Code Security sources: the lowest severity counted (critical, high, medium, low).
    min_severity TEXT NOT NULL DEFAULT 'high' CHECK (min_severity IN ('critical', 'high', 'medium', 'low')),
    -- A blocking rule fails the PR check; a non-blocking one only reports.
    blocking     BOOLEAN NOT NULL DEFAULT true,
    enabled      BOOLEAN NOT NULL DEFAULT true,
    created_by   TEXT NOT NULL DEFAULT '',
    updated_by   TEXT NOT NULL DEFAULT '',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX pr_gate_rules_name ON pr_gate_rules (org_id, lower(name));
