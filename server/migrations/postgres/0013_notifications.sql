-- Log monitors and notification delivery.

ALTER TABLE monitors DROP CONSTRAINT IF EXISTS monitors_type_check;
ALTER TABLE monitors ADD CONSTRAINT monitors_type_check CHECK (type IN ('metric', 'apm', 'log'));

-- Where `@webhook-<name>` / `@slack-<name>` in a monitor message deliver to (per org).
CREATE TABLE notification_channels (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    -- The handle messages mention, without '@': webhook-oncall, slack-checkout-alerts.
    handle      TEXT NOT NULL,
    kind        TEXT NOT NULL CHECK (kind IN ('webhook', 'slack')),
    -- AES-256-GCM(OC_SECRET_KEY): incoming-webhook URLs are credentials.
    url_enc     TEXT NOT NULL,
    -- Webhooks: HMAC-SHA256 signing secret (X-ObserveCraft-Signature).
    secret_enc  TEXT,
    created_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, handle)
);

-- One per (monitor event, recipient); retried with backoff until sent or given up.
CREATE TABLE notification_deliveries (
    id              BIGSERIAL PRIMARY KEY,
    event_id        BIGINT NOT NULL REFERENCES monitor_events(id) ON DELETE CASCADE,
    org_id          TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    recipient       TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'sent', 'failed', 'unknown_recipient')),
    attempts        INTEGER NOT NULL DEFAULT 0,
    error           TEXT,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    sent_at         TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX notification_deliveries_due ON notification_deliveries (next_attempt_at) WHERE status = 'pending';
CREATE INDEX notification_deliveries_org ON notification_deliveries (org_id, created_at DESC);
