-- Synthetic Monitoring & Testing: test suites, the org's synthetics settings and global variables.
-- Managed through routes/synthetics_suites.rs and routes/synthetics_settings.rs.

-- A test suite groups tests; its state is the worst state of its tests, its uptime the share of
-- passing scheduled runs of its tests.
CREATE TABLE synthetic_suites (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    env         TEXT NOT NULL DEFAULT '',
    team        TEXT NOT NULL DEFAULT '',
    tags        TEXT[] NOT NULL DEFAULT '{}',
    created_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX synthetic_suites_org ON synthetic_suites (org_id);

-- The tests of a suite (a test can be in several suites).
CREATE TABLE synthetic_suite_tests (
    suite_id UUID NOT NULL REFERENCES synthetic_suites(id) ON DELETE CASCADE,
    test_id  UUID NOT NULL REFERENCES synthetic_tests(id) ON DELETE CASCADE,
    org_id   TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    position INT NOT NULL DEFAULT 0,
    PRIMARY KEY (suite_id, test_id)
);
CREATE INDEX synthetic_suite_tests_test ON synthetic_suite_tests (test_id);

-- Settings → Default Settings: the enforced tag rules and the defaults of the test editor.
CREATE TABLE synthetic_settings (
    org_id            TEXT PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    -- Every test needs a tag for each key of the default tags.
    enforce_tag_keys  BOOLEAN NOT NULL DEFAULT false,
    -- Every test needs an environment.
    enforce_env       BOOLEAN NOT NULL DEFAULT false,
    -- {"locations": [...], "tags": [...], "timeoutSeconds": 30, "frequencySeconds": 300,
    --  "retry": {"count": 0, "intervalMs": 300}, "minFailureDurationMinutes": 0}
    defaults          JSONB NOT NULL DEFAULT '{}',
    updated_by        UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Settings → Global Variables: values tests use as {{ NAME }} in URLs, hosts, headers, bodies and
-- credentials. Secure values are never returned by the API nor kept in results.
CREATE TABLE synthetic_variables (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    value       TEXT NOT NULL,
    secure      BOOLEAN NOT NULL DEFAULT false,
    description TEXT NOT NULL DEFAULT '',
    tags        TEXT[] NOT NULL DEFAULT '{}',
    created_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX synthetic_variables_name ON synthetic_variables (org_id, name);
