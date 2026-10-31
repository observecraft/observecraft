-- Session Replay (Digital Experience → Session Replay): playlists of recorded sessions and who watched
-- which replay. Sessions are referred to by their RUM session id (their recordings live in ClickHouse,
-- `otel.otel_rum_replay`, for 30 days).
CREATE TABLE rum_replay_playlists (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX rum_replay_playlists_org ON rum_replay_playlists (org_id, lower(name));

CREATE TABLE rum_replay_playlist_sessions (
    playlist_id UUID NOT NULL REFERENCES rum_replay_playlists(id) ON DELETE CASCADE,
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    session_id  TEXT NOT NULL,
    -- When the session started (epoch ms), to look its events up.
    started_at  BIGINT NOT NULL,
    added_by    UUID REFERENCES users(id) ON DELETE SET NULL,
    added_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (playlist_id, session_id)
);

CREATE TABLE rum_replay_watches (
    org_id     TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    started_at BIGINT NOT NULL,
    watched_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, user_id, session_id)
);
CREATE INDEX rum_replay_watches_session ON rum_replay_watches (org_id, session_id);
