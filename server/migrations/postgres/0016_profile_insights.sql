-- Review state of profiling insights (Profiles → Insights): insights are detected from the profiles
-- at query time; only what a person decided about one is stored. Unknown keys are "for_review".
CREATE TABLE profile_insight_states (
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    -- <kind>:<service>:<env>, stable across ranges.
    insight_key TEXT NOT NULL,
    status      TEXT NOT NULL CHECK (status IN ('for_review', 'reviewed', 'resolved', 'ignored')),
    updated_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, insight_key)
);
