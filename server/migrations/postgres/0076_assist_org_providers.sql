-- Assist: an organization's own model provider (Organization Settings → Assist AI), used instead of the
-- install's (OC_ASSIST_*). The API key is sealed with the server's secret key, like other credentials.
CREATE TABLE IF NOT EXISTS assist_org_providers (
    org_id      TEXT PRIMARY KEY REFERENCES orgs(id) ON DELETE CASCADE,
    -- anthropic | openai (any OpenAI-compatible server)
    provider    TEXT NOT NULL,
    model       TEXT,
    base_url    TEXT,
    fallbacks   BOOLEAN,
    api_key_enc TEXT,
    updated_by  TEXT NOT NULL DEFAULT '',
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
