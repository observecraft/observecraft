-- Integrations → Google Workspace: an org's connection to a Google Workspace account, polled by the server
-- (`crate::google_workspace`), which reads the Admin SDK Reports API's activities (login, admin, drive, token, saml,
-- groups, mobile, user_accounts, …) and sends each event as a log (`source:gsuite`, as Datadog's Google Workspace
-- integration sends them). A service account with domain-wide delegation for the Reports API's audit scope acts as an
-- administrator (`subject`). One connection per org; its `id` keys the poller's jobs (`cloud_crawl_jobs`, provider
-- `google_workspace`, one job per application, whose `collected_until` is that application's cursor). The service
-- account's JSON key is sealed.
CREATE TABLE IF NOT EXISTS google_workspace_connections (
    org_id        TEXT NOT NULL PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    id            UUID NOT NULL DEFAULT gen_random_uuid() UNIQUE,
    -- The service account (its key's client_email), shown on the page; the whole key is in key_enc.
    client_email  TEXT NOT NULL,
    key_enc       TEXT NOT NULL,
    -- The administrator the service account acts as (domain-wide delegation).
    subject       TEXT NOT NULL,
    -- The Reports API applications read (login, admin, drive, …).
    applications  TEXT[] NOT NULL DEFAULT '{}',
    -- The admin who saved it last (a user or an application key's owner).
    updated_by    UUID,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Per application: the backlog being read over several runs (Google answers newest first: the window's start and end,
-- and where the next run carries on reading back from, epoch ms) and when the last run re-read the late-arrival
-- window (Google delivers some applications' events hours late).
CREATE TABLE IF NOT EXISTS google_workspace_progress (
    org_id            TEXT NOT NULL,
    connection_id     UUID NOT NULL REFERENCES google_workspace_connections(id) ON DELETE CASCADE,
    application       TEXT NOT NULL,
    backlog_start_ms  BIGINT,
    backlog_until_ms  BIGINT,
    backlog_before_ms BIGINT,
    rescanned_ms      BIGINT,
    PRIMARY KEY (connection_id, application)
);

-- The activities already sent within the windows runs read again (a hash of the application, `id.uniqueQualifier`
-- and `id.time`, and the activity's time), so a re-read sends only what arrived late. Pruned past those windows.
CREATE TABLE IF NOT EXISTS google_workspace_seen (
    org_id        TEXT NOT NULL,
    connection_id UUID NOT NULL REFERENCES google_workspace_connections(id) ON DELETE CASCADE,
    application   TEXT NOT NULL,
    activity_key  BIGINT NOT NULL,
    activity_ms   BIGINT NOT NULL,
    PRIMARY KEY (connection_id, application, activity_key)
);
CREATE INDEX IF NOT EXISTS google_workspace_seen_time ON google_workspace_seen (connection_id, application, activity_ms);
