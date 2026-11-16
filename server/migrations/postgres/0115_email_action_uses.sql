-- Single-use links in emails (On-Call's Acknowledge / Resolve, routes/on_call_links.rs): the signed
-- token's nonce is recorded when the link is used; rows older than the links' 24 h are deleted.
CREATE TABLE email_action_uses (
    org_id  TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    nonce   TEXT NOT NULL,
    used_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, nonce)
);
CREATE INDEX email_action_uses_used ON email_action_uses (org_id, used_at);
