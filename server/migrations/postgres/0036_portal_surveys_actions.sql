-- Developer Portal → Surveys and Self-Service Actions. See routes/portal_surveys.rs and
-- routes/portal_actions.rs.

-- A developer survey: its questions, who is asked, when it accepts answers and who hears about them.
CREATE TABLE portal_surveys (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id         TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    title          TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 200),
    description    TEXT NOT NULL DEFAULT '',
    -- Template the survey started from (`developer_experience`, …); '' for a custom survey.
    template       TEXT NOT NULL DEFAULT '',
    -- [{"id", "kind": likert | single | multi | text | nps, "title", "description", "required", "options"}]
    questions      JSONB NOT NULL DEFAULT '[]',
    -- Team handles asked; empty for every member of the org.
    audience_teams TEXT[] NOT NULL DEFAULT '{}',
    anonymous      BOOLEAN NOT NULL DEFAULT false,
    status         TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'open', 'closed')),
    opens_at       TIMESTAMPTZ,
    closes_at      TIMESTAMPTZ,
    -- Notified of each response: notification channel handles (`slack-devex`) or member emails.
    notify         TEXT[] NOT NULL DEFAULT '{}',
    -- Mixed into respondent keys, so anonymous responses can't be matched to users from outside.
    salt           TEXT NOT NULL,
    created_by     UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, id)
);
CREATE INDEX portal_surveys_org ON portal_surveys (org_id);

-- One response per respondent and survey: `respondent_key` is SHA-256 of the survey's salt and the
-- user id. Anonymous responses keep no user id, only that key.
CREATE TABLE portal_survey_responses (
    id             BIGSERIAL PRIMARY KEY,
    org_id         TEXT NOT NULL,
    survey_id      UUID NOT NULL,
    user_id        UUID REFERENCES users(id) ON DELETE SET NULL,
    respondent_key TEXT NOT NULL,
    -- The respondent's teams when answering (response rate by team).
    teams          TEXT[] NOT NULL DEFAULT '{}',
    -- {"<question id>": 4 | "option" | ["option", …] | "text"}
    answers        JSONB NOT NULL,
    submitted_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (survey_id, respondent_key),
    FOREIGN KEY (org_id, survey_id) REFERENCES portal_surveys(org_id, id) ON DELETE CASCADE
);
CREATE INDEX portal_survey_responses_org ON portal_survey_responses (org_id);

-- A self-service action: a form whose inputs fill one step, an outbound HTTP request sequence or an
-- ObserveCraft operation (declare an incident, page a team, comment on an incident, run a test).
CREATE TABLE portal_actions (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id        TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name          TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    description   TEXT NOT NULL DEFAULT '',
    category      TEXT NOT NULL,
    -- Blueprint the action started from; '' when built from scratch.
    blueprint     TEXT NOT NULL DEFAULT '',
    owner_team    TEXT NOT NULL DEFAULT '',
    -- [{"key", "label", "kind": text | select | number | boolean, "required", "options", "default", "description"}]
    inputs        JSONB NOT NULL DEFAULT '[]',
    -- http | create_incident | page_on_call | incident_comment | synthetic_run
    step_kind     TEXT NOT NULL,
    -- The step's templates (requests for http, fields for the others).
    step          JSONB NOT NULL,
    -- AES-GCM sealed JSON object {"name": "value"} of the credentials `{{secrets.name}}` refers to.
    secrets_enc   TEXT,
    -- Team handles whose members may run it; empty for every standard user and admin.
    allowed_teams TEXT[] NOT NULL DEFAULT '{}',
    created_by    UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, name),
    UNIQUE (org_id, id)
);

CREATE TABLE portal_action_runs (
    id               BIGSERIAL PRIMARY KEY,
    org_id           TEXT NOT NULL,
    action_id        UUID NOT NULL,
    user_id          UUID REFERENCES users(id) ON DELETE SET NULL,
    user_name        TEXT NOT NULL,
    inputs           JSONB NOT NULL DEFAULT '{}',
    status           TEXT NOT NULL CHECK (status IN ('succeeded', 'failed')),
    -- HTTP status of the last request sent (none for internal steps or when nothing answered).
    status_code      INT,
    duration_ms      DOUBLE PRECISION NOT NULL DEFAULT 0,
    -- Start of the last response or the internal step's outcome, with credentials redacted.
    response_excerpt TEXT NOT NULL DEFAULT '',
    error            TEXT,
    -- Where the result lives (the new repository, pull request, incident or page).
    link             TEXT NOT NULL DEFAULT '',
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (org_id, action_id) REFERENCES portal_actions(org_id, id) ON DELETE CASCADE
);
CREATE INDEX portal_action_runs_action ON portal_action_runs (action_id, created_at DESC);
CREATE INDEX portal_action_runs_org ON portal_action_runs (org_id);
