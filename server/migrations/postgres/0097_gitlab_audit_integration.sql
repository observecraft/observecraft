-- Integrations → GitLab Audit Events: an org's connection to a GitLab instance (GitLab.com or self-managed), polled by
-- the server (`crate::gitlab_audit`), which sends its audit events (authentication, tokens, members, merge requests,
-- projects and groups, CI/CD settings) as logs (`source:gitlab-audit-events`). One connection per org; its `id` keys
-- the poller's job (`cloud_crawl_jobs`, provider `gitlab_audit`, scope `events`, whose `collected_until` is the events
-- cursor). The access token is sealed.
CREATE TABLE IF NOT EXISTS gitlab_audit_connections (
    org_id            TEXT NOT NULL PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    id                UUID NOT NULL DEFAULT gen_random_uuid() UNIQUE,
    -- https://gitlab.com or the self-managed instance's URL.
    base_url          TEXT NOT NULL,
    token_enc         TEXT NOT NULL,
    -- instance (an administrator's token: every event), group or project (that group's or project's events).
    scope             TEXT NOT NULL DEFAULT 'instance' CHECK (scope IN ('instance', 'group', 'project')),
    -- The group or project path (example-org, example-org/storefront-api); empty for the instance.
    scope_path        TEXT NOT NULL DEFAULT '',
    -- While a backlog larger than a run's pages is read (newest first, page by page over several runs): the window's
    -- end (epoch ms), where the cursor moves once the backlog is read, and where the next run carries on reading.
    backlog_until_ms  BIGINT,
    backlog_before_ms BIGINT,
    -- The admin who saved it last (a user or an application key's owner).
    updated_by        UUID,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
