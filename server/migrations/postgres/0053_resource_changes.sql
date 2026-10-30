-- Infrastructure → Resources → Changes: the last configuration seen of every resource in the
-- catalog, which the change job (routes/resource_changes.rs) compares each run with the current one.
-- `doc` is the normalized configuration (catalog fields and attributes, and for Kubernetes objects
-- their labels, annotations and spec); `hash` its SHA-256, so a run reads only hashes.
CREATE TABLE resource_snapshots (
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    resource_id TEXT NOT NULL,
    hash        TEXT NOT NULL,
    doc         JSONB NOT NULL,
    deleted     BOOLEAN NOT NULL DEFAULT false,
    seen_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, resource_id)
);

-- One row per org: when its next run is due and who runs it (several servers may), and whether its
-- first run, which only records the baseline, is done.
CREATE TABLE resource_change_runs (
    org_id        TEXT PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    next_run_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    claimed_until TIMESTAMPTZ,
    baselined     BOOLEAN NOT NULL DEFAULT false
);
