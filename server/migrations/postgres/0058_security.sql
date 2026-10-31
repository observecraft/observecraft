-- Security: Cloud SIEM detection rules, the signals they raise, suppressions, content packs, and the
-- public research feed (routes/security_*.rs, datadog_api/security_monitoring.rs).

-- Detection rules: Datadog's security monitoring rules (`/api/v2/security_monitoring/rules`,
-- Terraform `datadog_security_monitoring_rule`). The default rules of a content pack are
-- `is_default` and carry the pack's id; they can be enabled and disabled, not edited or deleted.
CREATE TABLE security_rules (
    org_id             TEXT NOT NULL,
    id                 TEXT NOT NULL,                     -- abc-def-ghi
    version            INT NOT NULL DEFAULT 1,
    name               TEXT NOT NULL,
    message            TEXT NOT NULL DEFAULT '',
    type               TEXT NOT NULL DEFAULT 'log_detection',
    is_enabled         BOOLEAN NOT NULL DEFAULT true,
    is_default         BOOLEAN NOT NULL DEFAULT false,
    pack               TEXT,                              -- content pack of a default rule
    default_rule_id    TEXT,                              -- the catalog id of a default rule
    queries            JSONB NOT NULL DEFAULT '[]',
    cases              JSONB NOT NULL DEFAULT '[]',
    options            JSONB NOT NULL DEFAULT '{}',
    filters            JSONB NOT NULL DEFAULT '[]',
    tags               TEXT[] NOT NULL DEFAULT '{}',
    has_extended_title BOOLEAN NOT NULL DEFAULT false,
    -- The rest of the document as sent (reference tables, group_signals_by, scheduling, …), kept for read-back.
    extra              JSONB NOT NULL DEFAULT '{}',
    creation_author    BIGINT NOT NULL DEFAULT 0,
    update_author      BIGINT NOT NULL DEFAULT 0,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Evaluation state: the last minute evaluated.
    evaluated_at       TIMESTAMPTZ,
    PRIMARY KEY (org_id, id)
);
CREATE UNIQUE INDEX security_rules_default ON security_rules (org_id, default_rule_id) WHERE default_rule_id IS NOT NULL;

-- Signals: one per rule, case group and occurrence. An open occurrence is extended while its group
-- keeps matching within the rule's keep-alive, up to its maximum duration.
CREATE TABLE security_signals (
    org_id          TEXT NOT NULL,
    id              TEXT NOT NULL,
    rule_id         TEXT NOT NULL,
    rule_name       TEXT NOT NULL,
    rule_type       TEXT NOT NULL DEFAULT 'log_detection',
    title           TEXT NOT NULL,
    message         TEXT NOT NULL DEFAULT '',
    severity        TEXT NOT NULL,                        -- info | low | medium | high | critical
    case_name       TEXT NOT NULL DEFAULT '',
    condition       TEXT NOT NULL DEFAULT '',
    group_key       TEXT NOT NULL DEFAULT '',
    group_values    JSONB NOT NULL DEFAULT '{}',          -- {"@usr.email": "…"}
    queries         JSONB NOT NULL DEFAULT '[]',          -- the rule's queries when raised (name, query)
    source          TEXT NOT NULL DEFAULT '',             -- log source (cloudtrail, okta, …)
    services        TEXT[] NOT NULL DEFAULT '{}',
    env             TEXT NOT NULL DEFAULT '',
    tags            TEXT[] NOT NULL DEFAULT '{}',
    -- Who and what: the top values of the matching logs (IPs, users, hosts) with counts.
    entities        JSONB NOT NULL DEFAULT '{}',
    events_matched  BIGINT NOT NULL DEFAULT 0,
    first_seen      TIMESTAMPTZ NOT NULL,
    last_seen       TIMESTAMPTZ NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Triage.
    state           TEXT NOT NULL DEFAULT 'open',         -- open | under_review | archived
    archive_reason  TEXT NOT NULL DEFAULT 'none',
    archive_comment TEXT NOT NULL DEFAULT '',
    assignee_id     UUID,
    state_updated_at TIMESTAMPTZ,
    state_updated_by UUID,
    incident_ids    BIGINT[] NOT NULL DEFAULT '{}',
    suppressed      BOOLEAN NOT NULL DEFAULT false,
    PRIMARY KEY (org_id, id)
);
CREATE INDEX security_signals_time ON security_signals (org_id, created_at DESC);
CREATE INDEX security_signals_rule ON security_signals (org_id, rule_id, group_key, last_seen DESC);

-- Suppressions (`/api/v2/security_monitoring/configuration/suppressions`, Terraform
-- `datadog_security_monitoring_suppression`): signals of the rules matching `rule_query` whose
-- group or tags match `suppression_query` are not raised; logs matching `data_exclusion_query` are
-- left out of the evaluation.
CREATE TABLE security_suppressions (
    org_id               TEXT NOT NULL,
    id                   TEXT NOT NULL,
    version              INT NOT NULL DEFAULT 1,
    name                 TEXT NOT NULL,
    description          TEXT NOT NULL DEFAULT '',
    enabled              BOOLEAN NOT NULL DEFAULT true,
    rule_query           TEXT NOT NULL DEFAULT '',
    suppression_query    TEXT NOT NULL DEFAULT '',
    data_exclusion_query TEXT NOT NULL DEFAULT '',
    start_date           BIGINT,
    expiration_date      BIGINT,
    tags                 TEXT[] NOT NULL DEFAULT '{}',
    creator              TEXT NOT NULL DEFAULT '',
    updater              TEXT NOT NULL DEFAULT '',
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, id)
);

-- Content packs an org turned off (they are on by default; their rules follow).
CREATE TABLE security_content_packs (
    org_id     TEXT NOT NULL,
    pack       TEXT NOT NULL,
    enabled    BOOLEAN NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, pack)
);

-- Research feed: CISA's Known Exploited Vulnerabilities catalog (public domain), shared by every
-- org like the advisories of Code Security. Holds only what the catalog publishes, never org data.
CREATE TABLE security_kev (
    cve_id            TEXT PRIMARY KEY,
    vendor            TEXT NOT NULL DEFAULT '',
    product           TEXT NOT NULL DEFAULT '',
    name              TEXT NOT NULL DEFAULT '',
    description       TEXT NOT NULL DEFAULT '',
    required_action   TEXT NOT NULL DEFAULT '',
    ransomware        BOOLEAN NOT NULL DEFAULT false,
    cwes              TEXT[] NOT NULL DEFAULT '{}',
    date_added        DATE NOT NULL,
    due_date          DATE,
    fetched_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX security_kev_added ON security_kev (date_added DESC);
