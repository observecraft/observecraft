-- Incident detail views: timeline importance + authorship, response team roles, pages, notifications,
-- linked documents and postmortems.

ALTER TABLE incident_events
    ADD COLUMN important BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN author_id UUID REFERENCES users(id) ON DELETE SET NULL;

ALTER TABLE incident_responders
    ADD COLUMN added_at TIMESTAMPTZ NOT NULL DEFAULT now();
UPDATE incident_responders r SET added_at = i.declared_at FROM incidents i WHERE i.id = r.incident_id;

-- Pages sent to a team's on-call (delivery needs an on-call integration; status is tracked here).
CREATE TABLE incident_pages (
    id               BIGSERIAL PRIMARY KEY,
    incident_id      BIGINT NOT NULL REFERENCES incidents(id) ON DELETE CASCADE,
    team_handle      TEXT NOT NULL,
    title            TEXT NOT NULL,
    status           TEXT NOT NULL DEFAULT 'triggered' CHECK (status IN ('triggered', 'acknowledged', 'resolved')),
    created_by       TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    acknowledged_by  TEXT,
    acknowledged_at  TIMESTAMPTZ,
    resolved_at      TIMESTAMPTZ
);
CREATE INDEX incident_pages_incident ON incident_pages (incident_id);

-- Stakeholder notifications (email addresses, #channels, @handles).
CREATE TABLE incident_notifications (
    id          BIGSERIAL PRIMARY KEY,
    incident_id BIGINT NOT NULL REFERENCES incidents(id) ON DELETE CASCADE,
    recipients  TEXT[] NOT NULL,
    subject     TEXT NOT NULL,
    body        TEXT NOT NULL DEFAULT '',
    automated   BOOLEAN NOT NULL DEFAULT false,
    author      TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX incident_notifications_incident ON incident_notifications (incident_id, created_at DESC);

-- Existing notification timeline entries become notification records.
INSERT INTO incident_notifications (incident_id, recipients, subject, body, automated, author, created_at)
SELECT e.incident_id,
       COALESCE((SELECT array_agg(m[1]) FROM regexp_matches(e.body, '(#[A-Za-z0-9_-]+)', 'g') AS m), '{}'),
       split_part(e.body, ' — ', 1),
       '', true, e.author, e.created_at
FROM incident_events e WHERE e.kind = 'notification';

CREATE TABLE incident_documents (
    id          BIGSERIAL PRIMARY KEY,
    incident_id BIGINT NOT NULL REFERENCES incidents(id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    url         TEXT NOT NULL,
    created_by  TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE incident_postmortems (
    incident_id BIGINT PRIMARY KEY REFERENCES incidents(id) ON DELETE CASCADE,
    template    TEXT NOT NULL,
    content     TEXT NOT NULL,
    created_by  TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
