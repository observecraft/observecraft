-- Cloud Security's account scanning (docs/adr/0006-cloud-posture-scanning.md): Prowler, run by the `scanner`
-- deployment, checks the cloud accounts whose posture collection is on (AWS `resources_config`
-- `cloud_security_posture_management_collection`).

-- One run of the scanner over an account. The server queues them, a scanner claims one (with a lease it renews
-- while Prowler runs) and uploads its OCSF results.
CREATE TABLE IF NOT EXISTS posture_scans (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id         TEXT NOT NULL,
    provider       TEXT NOT NULL CHECK (provider IN ('aws', 'azure', 'gcp')),
    -- The integration's account row (aws_accounts.id) and the provider's id of the account.
    account_ref    UUID NOT NULL,
    account_id     TEXT NOT NULL,
    status         TEXT NOT NULL DEFAULT 'queued' CHECK (status IN ('queued', 'running', 'done', 'failed')),
    scanner        TEXT,
    lease_until    TIMESTAMPTZ,
    queued_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at     TIMESTAMPTZ,
    finished_at    TIMESTAMPTZ,
    error          TEXT,
    -- What ran (Prowler's version) and what it found.
    tool_version   TEXT,
    passed         INTEGER,
    failed         INTEGER
);
CREATE INDEX IF NOT EXISTS posture_scans_queue ON posture_scans (status, queued_at);
CREATE INDEX IF NOT EXISTS posture_scans_account ON posture_scans (org_id, account_ref, queued_at DESC);
-- At most one queued or running scan per account.
CREATE UNIQUE INDEX IF NOT EXISTS posture_scans_open ON posture_scans (account_ref) WHERE status IN ('queued', 'running');

-- The checks' descriptions, from the scanner's own metadata (the same for every org). Kept as the results bring them,
-- so they follow the pinned Prowler version.
CREATE TABLE IF NOT EXISTS posture_checks (
    provider      TEXT NOT NULL,
    check_id      TEXT NOT NULL,
    title         TEXT NOT NULL,
    severity      TEXT NOT NULL,
    service       TEXT NOT NULL DEFAULT '',
    resource_type TEXT NOT NULL DEFAULT '',
    description   TEXT NOT NULL DEFAULT '',
    risk          TEXT NOT NULL DEFAULT '',
    remediation   TEXT NOT NULL DEFAULT '',
    reference_urls JSONB NOT NULL DEFAULT '[]',
    -- Framework → requirement ids (CIS-2.0 → ["1.4", …]).
    compliance    JSONB NOT NULL DEFAULT '{}',
    tool_version  TEXT NOT NULL DEFAULT '',
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (provider, check_id)
);

-- The latest evaluation of each check on each resource of an account. A resource's rows go when a later scan of
-- the account no longer reports it.
CREATE TABLE IF NOT EXISTS posture_findings (
    id                    UUID NOT NULL DEFAULT gen_random_uuid(),
    org_id                TEXT NOT NULL,
    provider              TEXT NOT NULL,
    account_ref           UUID NOT NULL,
    account_id            TEXT NOT NULL,
    check_id              TEXT NOT NULL,
    resource_uid          TEXT NOT NULL,
    resource_name         TEXT NOT NULL DEFAULT '',
    resource_type         TEXT NOT NULL DEFAULT '',
    region                TEXT NOT NULL DEFAULT '',
    evaluation            TEXT NOT NULL CHECK (evaluation IN ('pass', 'fail')),
    severity              TEXT NOT NULL,
    -- Prowler's explanation for this resource ("S3 Bucket example-shop-assets has public access").
    detail                TEXT NOT NULL DEFAULT '',
    resource_tags         JSONB NOT NULL DEFAULT '[]',
    first_seen_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    evaluation_changed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_scan             UUID,
    muted                 BOOLEAN NOT NULL DEFAULT false,
    mute_reason           TEXT,
    mute_description      TEXT,
    mute_expire_at        TIMESTAMPTZ,
    muted_by              TEXT,
    muted_at              TIMESTAMPTZ,
    PRIMARY KEY (org_id, id),
    UNIQUE (org_id, account_ref, check_id, resource_uid)
);
CREATE INDEX IF NOT EXISTS posture_findings_open ON posture_findings (org_id, evaluation, severity);
