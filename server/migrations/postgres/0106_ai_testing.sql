-- AI Testing (Synthetic Monitoring → AI Testing): jobs are assist_runs of kind 'testing' whose agent explores a site in
-- oc-browser and proposes browser tests. Proposals wait for review here; accepting one creates its synthetic test (and
-- its suite). Agent profiles lend a job credentials as references to global variables (never the values).
CREATE TABLE ai_testing_profiles (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    -- [{"usage": "username" | "email" | "password" | "otp" | "other", "variable": "SHOP_USER"}]
    variables   JSONB NOT NULL DEFAULT '[]',
    created_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, name)
);

CREATE TABLE ai_testing_proposals (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id       TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    run_id       UUID NOT NULL,
    suite        TEXT NOT NULL,
    name         TEXT NOT NULL,
    description  TEXT NOT NULL DEFAULT '',
    start_url    TEXT NOT NULL,
    device_id    TEXT NOT NULL DEFAULT '',
    -- Browser test steps (Datadog's browser step model), made of the agent's recorded actions.
    steps        JSONB NOT NULL,
    -- The sample run (SyntheticRunDetail, without screenshots).
    verification JSONB,
    status       TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'accepted', 'rejected')),
    test_id      UUID REFERENCES synthetic_tests(id) ON DELETE SET NULL,
    feedback     TEXT NOT NULL DEFAULT '',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    decided_at   TIMESTAMPTZ,
    decided_by   UUID REFERENCES users(id) ON DELETE SET NULL
);
CREATE INDEX ai_testing_proposals_review ON ai_testing_proposals (org_id, status, created_at DESC);
CREATE INDEX ai_testing_proposals_run ON ai_testing_proposals (org_id, run_id);
