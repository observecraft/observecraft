-- Developer Portal → Home and Settings: source connectors (GitHub, GitLab, Jira, Linear), each
-- member's identity on those providers, app shortcuts and favourite entities. See
-- routes/portal_home.rs and routes/portal_sources.rs.

-- One connector per provider and org, configured by admins. The credential is sealed with the
-- server's secret key (auth::crypto::SecretBox) and never returned to clients.
CREATE TABLE portal_connectors (
    org_id           TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    provider         TEXT NOT NULL CHECK (provider IN ('github', 'gitlab', 'jira', 'linear')),
    -- API base URL (GitHub Enterprise, self-managed GitLab, the Jira site).
    base_url         TEXT NOT NULL,
    -- Jira: the email the API token belongs to; '' otherwise.
    account          TEXT NOT NULL DEFAULT '',
    secret_enc       TEXT NOT NULL,
    last_checked_at  TIMESTAMPTZ,
    last_check_ok    BOOLEAN,
    last_check_error TEXT NOT NULL DEFAULT '',
    updated_by       UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, provider)
);

-- Who a member is on each provider: GitHub / GitLab username, Jira / Linear email.
CREATE TABLE portal_identities (
    org_id     TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider   TEXT NOT NULL CHECK (provider IN ('github', 'gitlab', 'jira', 'linear')),
    handle     TEXT NOT NULL CHECK (length(handle) BETWEEN 1 AND 320),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, user_id, provider)
);

-- Shortcuts to the org's tools shown on the portal's home page.
CREATE TABLE portal_apps (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id     TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name       TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    url        TEXT NOT NULL CHECK (length(url) BETWEEN 1 AND 2048),
    -- Key of the web app's logo registry (`github`, `tile:servicenow`, …); '' for a generic icon.
    logo       TEXT NOT NULL DEFAULT '',
    position   INT NOT NULL DEFAULT 0,
    created_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX portal_apps_org ON portal_apps (org_id, position);

-- Entities (services) a member starred on the home page.
CREATE TABLE portal_favorites (
    org_id     TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    entity     TEXT NOT NULL CHECK (length(entity) BETWEEN 1 AND 200),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, user_id, entity)
);
