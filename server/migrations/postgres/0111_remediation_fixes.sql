-- Remediation (ADR-0008, level Fix in code): fixes proposed for security findings and the merge requests opened
-- from them, so a finding shows its fix until the merge request merges (the next scan then closes the finding).
-- A library vulnerability's fix is a manifest bump made at once (`change`); a static finding's is an Assist run of
-- kind 'fix' (`run_id`, its change on assist_runs). See routes/remediation.rs.
CREATE TABLE IF NOT EXISTS remediation_fixes (
    org_id        TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    id            UUID NOT NULL DEFAULT gen_random_uuid(),
    -- library | code | cloud_misconfiguration | cloud_vulnerability
    finding_kind  TEXT NOT NULL,
    -- A library vulnerability's key, a static finding's id.
    finding_id    TEXT NOT NULL,
    -- As Code Security names it: host/owner/name.
    repository    TEXT NOT NULL,
    -- manifest | assist
    method        TEXT NOT NULL,
    run_id        UUID,
    -- {title, description, files: [{path, before, after}]}
    change        JSONB,
    -- What must still happen (regenerate the lockfile, rotate the credential); also in the merge request.
    notes         TEXT[] NOT NULL DEFAULT '{}',
    -- Why no change was proposed, and what to do instead.
    explanation   TEXT,
    -- Why the connection can't open a merge request (a missing scope): the diff is copied instead.
    blocked       TEXT,
    -- {url, number, branch, draft, state: open | merged | closed}
    merge_request JSONB,
    created_by    TEXT NOT NULL DEFAULT '',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, id)
);
CREATE INDEX IF NOT EXISTS remediation_fixes_finding ON remediation_fixes (org_id, finding_kind, finding_id, created_at DESC);
