-- Assist Code: the repositories sessions may read and change (through the org's GitHub or GitLab
-- connection), the org's pull request defaults, and on each coding run the change it proposed and the
-- pull request opened from it. Code automations are agents of kind 'code' on a repository.
CREATE TABLE IF NOT EXISTS assist_code_repos (
    org_id   TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    -- owner/name (GitLab: group/…/name)
    repo     TEXT NOT NULL,
    -- github | gitlab
    provider TEXT NOT NULL,
    -- Services whose code lives there.
    services TEXT[] NOT NULL DEFAULT '{}',
    added_by TEXT NOT NULL DEFAULT '',
    added_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, repo)
);

CREATE TABLE IF NOT EXISTS assist_code_settings (
    org_id            TEXT PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    branch_prefix     TEXT NOT NULL DEFAULT 'assist/',
    -- Pull requests open as drafts.
    draft             BOOLEAN NOT NULL DEFAULT true,
    -- Open the pull request as soon as a session proposes a change (else on the reviewer's click).
    auto_pull_request BOOLEAN NOT NULL DEFAULT false,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE assist_runs ADD COLUMN IF NOT EXISTS change JSONB;
ALTER TABLE assist_runs ADD COLUMN IF NOT EXISTS pull_request JSONB;

ALTER TABLE assist_agents ADD COLUMN IF NOT EXISTS kind TEXT NOT NULL DEFAULT 'agent';
ALTER TABLE assist_agents ADD COLUMN IF NOT EXISTS repo TEXT;
