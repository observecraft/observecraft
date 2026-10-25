-- Flaky test management: the state a team gives a flaky test (detection itself is computed from
-- CI test spans in ClickHouse). Tests are keyed by repository, suite and name.
CREATE TABLE ci_test_states (
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    repository  TEXT NOT NULL,
    suite       TEXT NOT NULL,
    name        TEXT NOT NULL,
    state       TEXT NOT NULL CHECK (state IN ('active', 'quarantined', 'disabled', 'fixed')),
    updated_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, repository, suite, name)
);
