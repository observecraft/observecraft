-- Status pages (Incident Response → Status Pages): the state of an org's components for its users,
-- and the notices posted about incidents that affect them. See routes/status_pages.rs.
--
-- A page's slug is unique across the instance, so the read-only rendering at /status/<slug> finds a
-- page without knowing its org. Public pages render there for anyone; internal pages only for
-- signed-in members of the page's org.

CREATE TABLE status_pages (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    -- Lowercase letters, digits and dashes, neither first nor last (`shop-status`).
    slug        TEXT NOT NULL UNIQUE CHECK (slug ~ '^[a-z0-9]([a-z0-9-]{0,62}[a-z0-9])?$'),
    visibility  TEXT NOT NULL CHECK (visibility IN ('public', 'internal')),
    description TEXT NOT NULL DEFAULT '',
    -- An http(s) image shown in the header instead of the name; '' for none.
    logo_url    TEXT NOT NULL DEFAULT '',
    -- The page shows a subscribe button for these channels. Subscriptions aren't delivered yet.
    email_subscriptions BOOLEAN NOT NULL DEFAULT false,
    slack_subscriptions BOOLEAN NOT NULL DEFAULT false,
    -- The components in display order: [{"id": "c_…", "name": "Checkout", "group": "Storefront",
    -- "status": "operational"}]. `status` is the one set by hand; open notices can make it worse.
    components  JSONB NOT NULL DEFAULT '[]',
    created_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Target of the notices' (org, page) key.
    UNIQUE (org_id, id)
);

CREATE INDEX status_pages_org ON status_pages (org_id, name);

CREATE TABLE status_page_notices (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      TEXT NOT NULL,
    page_id     UUID NOT NULL,
    title       TEXT NOT NULL,
    -- The status of the latest update.
    status      TEXT NOT NULL CHECK (status IN ('investigating', 'identified', 'monitoring', 'resolved')),
    -- The impacts of the latest update: [{"componentId": "c_…", "impact": "partial_outage"}].
    impacts     JSONB NOT NULL DEFAULT '[]',
    -- Optional link to the incident (its IR number) in the same org.
    incident_id INTEGER,
    created_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    started_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at TIMESTAMPTZ,
    UNIQUE (org_id, id),
    FOREIGN KEY (org_id, page_id) REFERENCES status_pages(org_id, id) ON DELETE CASCADE
);

CREATE INDEX status_page_notices_page ON status_page_notices (org_id, page_id, started_at DESC);

-- Every update posted to a notice, the first one included; the uptime bars replay them.
CREATE TABLE status_page_notice_updates (
    id          BIGSERIAL PRIMARY KEY,
    org_id      TEXT NOT NULL,
    notice_id   UUID NOT NULL,
    status      TEXT NOT NULL CHECK (status IN ('investigating', 'identified', 'monitoring', 'resolved')),
    -- Markdown.
    message     TEXT NOT NULL,
    -- The impacts from this update on, in the notice's format.
    impacts     JSONB NOT NULL DEFAULT '[]',
    created_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (org_id, notice_id) REFERENCES status_page_notices(org_id, id) ON DELETE CASCADE
);

CREATE INDEX status_page_notice_updates_notice ON status_page_notice_updates (org_id, notice_id, created_at);
