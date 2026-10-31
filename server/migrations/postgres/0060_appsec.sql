-- App & API Protection (routes/appsec*.rs, datadog_api/appsec.rs): the In-App WAF configuration the
-- Datadog API and Terraform provider manage (custom rules, exclusion filters, policies), the denylist,
-- and per-org settings (custom deny responses, sensitive data scanners). Security traces, endpoints
-- and attackers are read from the spans.

-- WAF custom rules, exclusion filters (the passlist) and policies, as Datadog documents them:
-- `attributes` holds the document's attributes as sent (minus the metadata the server sets).
CREATE TABLE appsec_waf_objects (
    org_id      TEXT NOT NULL,
    kind        TEXT NOT NULL,                       -- custom_rule | exclusion_filter | policy
    id          UUID NOT NULL DEFAULT gen_random_uuid(),
    attributes  JSONB NOT NULL,
    added_by    TEXT NOT NULL DEFAULT '',
    added_by_name TEXT NOT NULL DEFAULT '',
    added_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    modified_by TEXT,
    modified_by_name TEXT,
    modified_at TIMESTAMPTZ,
    PRIMARY KEY (org_id, kind, id)
);

-- Blocked IPs and users. Blocking itself is applied by tracers through Remote Configuration, which
-- ObserveCraft doesn't serve yet: the list is kept and shown.
CREATE TABLE appsec_denylist (
    org_id      TEXT NOT NULL,
    kind        TEXT NOT NULL,                       -- ip | user
    value       TEXT NOT NULL,
    reason      TEXT NOT NULL DEFAULT '',
    source      TEXT NOT NULL DEFAULT 'Attackers Explorer',
    expires_at  TIMESTAMPTZ,
    created_by  TEXT NOT NULL DEFAULT '',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, kind, value)
);

-- Small per-org settings: `custom_response` ({mode, status, redirect_url, redirect_status}),
-- `scanners` ({id: enabled}), `blocking_allowed` (bool).
CREATE TABLE appsec_settings (
    org_id     TEXT NOT NULL,
    key        TEXT NOT NULL,
    value      JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, key)
);
