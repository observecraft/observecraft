-- Tagging policies and where resources come from (docs/adr/0008-remediation-and-provenance.md, decisions 1-3):
-- the tags the resources of a scope must carry, their coverage over time, and the optional Terraform state sources
-- that resolve untagged cloud resources. See routes/tagging_policies.rs, routes/tagging_rules.rs and
-- routes/provenance.rs.

CREATE TABLE tagging_policies (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name        TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    description TEXT NOT NULL DEFAULT '',
    -- cloud | kubernetes | image | service
    target      TEXT NOT NULL CHECK (target IN ('cloud', 'kubernetes', 'image', 'service')),
    -- {"provider": "aws", "account": "1234*", "cluster": "", "resourceType": "", "names": ""}: comma-separated patterns.
    scope       JSONB NOT NULL DEFAULT '{}',
    -- [{"key": "tf_repo|git_repo", "allowedValues": []}, …]
    required    JSONB NOT NULL,
    enabled     BOOLEAN NOT NULL DEFAULT true,
    -- The recommended policy it was installed or created from, if any.
    recommended TEXT,
    created_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, name)
);

-- One row per org: the recommended set is installed once (deleting a policy keeps it deleted), and the daily
-- coverage snapshots are scheduled. Workers claim due orgs with FOR UPDATE SKIP LOCKED.
CREATE TABLE tagging_policy_settings (
    org_id                 TEXT PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    recommended_installed  TIMESTAMPTZ NOT NULL DEFAULT now(),
    next_snapshot_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    claimed_until          TIMESTAMPTZ
);
CREATE INDEX tagging_policy_settings_due ON tagging_policy_settings (next_snapshot_at);

-- Coverage per UTC day (the day's latest evaluation): the total (dimension 'total', key ''), and per policy (its id),
-- target, account, cluster and team.
CREATE TABLE tagging_coverage_snapshots (
    org_id    TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    day       DATE NOT NULL,
    dimension TEXT NOT NULL CHECK (dimension IN ('total', 'policy', 'target', 'account', 'cluster', 'team')),
    key       TEXT NOT NULL,
    evaluated INT NOT NULL,
    compliant INT NOT NULL,
    taken_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, dimension, key, day)
);
CREATE INDEX tagging_coverage_snapshots_day ON tagging_coverage_snapshots (org_id, day);

-- A read-only Terraform state an org connects, mapping cloud ids to resource addresses for resources without
-- provenance tags. Reading it (S3, Azure Blob, GCS with the cloud integrations' credentials, or an HTTP backend) is
-- not built yet; the resolver's index (provenance::parse_state) is.
CREATE TABLE terraform_state_sources (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id     TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name       TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    -- s3 | azurerm | gcs | http
    backend    TEXT NOT NULL CHECK (backend IN ('s3', 'azurerm', 'gcs', 'http')),
    -- s3://bucket/key, https://account.blob.core.windows.net/container/key, gs://bucket/prefix, https://…
    location   TEXT NOT NULL,
    -- The repository and directory whose configuration the state belongs to.
    repo       TEXT NOT NULL,
    path       TEXT NOT NULL DEFAULT '',
    enabled    BOOLEAN NOT NULL DEFAULT true,
    last_read_at TIMESTAMPTZ,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, name)
);
