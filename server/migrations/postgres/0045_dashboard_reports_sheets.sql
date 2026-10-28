-- Dashboards → Reports and Dashboards → Sheets. Managed through routes/dashboard_reports.rs and
-- routes/sheets.rs.

-- A scheduled dashboard report: a dashboard (with template variable values) summarized over a
-- time frame and sent to recipients (member emails and notification channel handles) on a
-- schedule in an IANA time zone.
CREATE TABLE dashboard_reports (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id          TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    dashboard_id    TEXT NOT NULL REFERENCES dashboards(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    -- {"env": "prod", "service": "*"}: template variable name → value.
    template_values JSONB NOT NULL DEFAULT '{}',
    -- 1h | 4h | 1d | 2d | 1w | 1mo: the window that ends when the report is sent.
    time_frame      TEXT NOT NULL,
    -- {"frequency": "weekly", "time": "09:00", "weekday": 1, "monthDay": null, "timezone": "Europe/Paris"}
    schedule        JSONB NOT NULL,
    -- `ops@example.com` (an org member) or a channel handle (`slack-checkout`, `webhook-ops`).
    recipients      TEXT[] NOT NULL DEFAULT '{}',
    message         TEXT NOT NULL DEFAULT '',
    author_id       UUID REFERENCES users(id) ON DELETE SET NULL,
    next_run_at     TIMESTAMPTZ,
    -- Set while one server sends the report, so several servers can run the scheduler.
    claimed_until   TIMESTAMPTZ,
    last_sent_at    TIMESTAMPTZ,
    -- sent | partial | failed: the last send's outcome; last_error explains the recipients that failed.
    last_status     TEXT,
    last_error      TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX dashboard_reports_org ON dashboard_reports (org_id, dashboard_id);
CREATE INDEX dashboard_reports_due ON dashboard_reports (next_run_at);

-- Members who also receive a report by email.
CREATE TABLE dashboard_report_subscriptions (
    report_id   UUID NOT NULL REFERENCES dashboard_reports(id) ON DELETE CASCADE,
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (report_id, user_id)
);
CREATE INDEX dashboard_report_subscriptions_user ON dashboard_report_subscriptions (org_id, user_id);

-- A spreadsheet: tabs ("sheets") backed by a data source query (metrics, logs, a reference table)
-- with calculated columns, sort, filters and a pivot. The definition is the JSON the web app edits
-- (web/src/api/sheets/types.ts); the server checks only its size.
CREATE TABLE spreadsheets (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    team        TEXT,
    author_id   UUID REFERENCES users(id) ON DELETE SET NULL,
    definition  JSONB NOT NULL DEFAULT '{"sheets": []}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    modified_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX spreadsheets_org ON spreadsheets (org_id, modified_at DESC);
