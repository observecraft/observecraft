-- "Connect" of connector packages using OAuth's authorization code flow (crates/oc-api/src/routes/connector_oauth.rs):
-- one row per consent page opened, used once by the callback within 10 minutes. It names the org, the admin, the
-- connector and the connection being reconnected; `input` is what the admin entered (name, config, streams),
-- `secrets` the secrets entered, each sealed, and `verifier` the PKCE verifier, sealed.
CREATE TABLE IF NOT EXISTS connector_oauth_states (
    state         TEXT PRIMARY KEY,
    org_id        TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    user_id       UUID NOT NULL,
    connector_id  TEXT NOT NULL,
    connection_id UUID,
    input         JSONB NOT NULL DEFAULT '{}',
    secrets       JSONB NOT NULL DEFAULT '{}',
    verifier      TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
