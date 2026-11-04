-- Software Delivery → GitLab CI: an org's connection to GitLab (GitLab.com or self-managed), polled by the
-- server (`crate::gitlab`), which turns finished pipelines into CI/CD traces. One connection per org; its `id`
-- keys the poller's jobs (`cloud_crawl_jobs`, provider `gitlab`: a discovery job lists the projects to follow,
-- then one job per project whose `collected_until` is the pipelines cursor). The token is sealed.
CREATE TABLE IF NOT EXISTS gitlab_ci_connections (
    org_id            TEXT NOT NULL PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    id                UUID NOT NULL DEFAULT gen_random_uuid() UNIQUE,
    -- https://gitlab.com or the self-managed instance's URL.
    base_url          TEXT NOT NULL,
    token_enc         TEXT NOT NULL,
    -- A group (with or without its subgroups) and/or explicit project paths to follow.
    group_path        TEXT NOT NULL DEFAULT '',
    include_subgroups BOOLEAN NOT NULL DEFAULT true,
    project_paths     TEXT[] NOT NULL DEFAULT '{}',
    -- The admin who saved it last (a user or an application key's owner).
    updated_by        UUID,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Pipelines already exported (a pipeline is sent once, after it finished); rows older than the CI table's
-- retention are pruned by the poller.
CREATE TABLE IF NOT EXISTS gitlab_ci_exported (
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    project_id  BIGINT NOT NULL,
    pipeline_id BIGINT NOT NULL,
    jobs        INTEGER NOT NULL DEFAULT 0,
    exported_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, project_id, pipeline_id)
);
