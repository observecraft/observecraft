-- What people decided about Error Tracking issues. Issues are grouped from error logs at query time
-- (their id is stable: service, error type and message pattern); only the review state is stored.
-- Unknown issues are "for_review"; a resolved issue seen again after updated_at regresses.
CREATE TABLE error_issue_states (
    org_id     TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    issue_id   TEXT NOT NULL,
    status     TEXT NOT NULL CHECK (status IN ('for_review', 'reviewed', 'resolved', 'ignored')),
    updated_by UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, issue_id)
);
