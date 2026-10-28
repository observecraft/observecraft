-- On-Call (Incident Response → On-Call): schedules, escalation policies, team routing, pages and
-- each member's notification preferences. Pages are delivered through the notification channels
-- of monitors (email, Slack, webhooks); an escalation worker claims due pages with
-- FOR UPDATE SKIP LOCKED, so several servers notify once.

-- Rotations of responders; `layers` is a JSON array (see routes/on_call_schedule.rs).
CREATE TABLE on_call_schedules (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id         TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name           TEXT NOT NULL,
    time_zone      TEXT NOT NULL DEFAULT 'UTC',
    team_handle    TEXT,
    tags           TEXT[] NOT NULL DEFAULT '{}',
    layers         JSONB NOT NULL DEFAULT '[]',
    -- At most this many responders at a time in the final schedule (NULL: everyone on call in any layer).
    max_concurrent INT CHECK (max_concurrent BETWEEN 1 AND 10),
    created_by     UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_by     UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX on_call_schedules_org ON on_call_schedules (org_id, lower(name));

-- Someone else covers part of a schedule.
CREATE TABLE on_call_overrides (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    schedule_id UUID NOT NULL REFERENCES on_call_schedules(id) ON DELETE CASCADE,
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    start_at    TIMESTAMPTZ NOT NULL,
    end_at      TIMESTAMPTZ NOT NULL CHECK (end_at > start_at),
    created_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX on_call_overrides_schedule ON on_call_overrides (schedule_id, end_at);

-- `steps`: [{"targets": [{"kind": "user|schedule|team", "id": "…"}], "escalateAfterMinutes": 5}, …]
CREATE TABLE on_call_escalation_policies (
    id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id                TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name                  TEXT NOT NULL,
    team_handle           TEXT,
    tags                  TEXT[] NOT NULL DEFAULT '{}',
    steps                 JSONB NOT NULL,
    -- Start over from the first step this many times after the last one.
    repeat_count          INT NOT NULL DEFAULT 0 CHECK (repeat_count BETWEEN 0 AND 10),
    resolve_after_minutes INT CHECK (resolve_after_minutes BETWEEN 1 AND 43200),
    created_by            UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_by            UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX on_call_policies_org ON on_call_escalation_policies (org_id, lower(name));

-- Which escalation policy a team's pages follow.
CREATE TABLE on_call_team_routing (
    org_id      TEXT NOT NULL,
    team_handle TEXT NOT NULL,
    policy_id   UUID NOT NULL REFERENCES on_call_escalation_policies(id) ON DELETE CASCADE,
    updated_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, team_handle),
    FOREIGN KEY (org_id, team_handle) REFERENCES teams(org_id, handle) ON DELETE CASCADE
);

CREATE TABLE on_call_pages (
    id                 BIGSERIAL PRIMARY KEY,
    org_id             TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    title              TEXT NOT NULL,
    description        TEXT NOT NULL DEFAULT '',
    urgency            TEXT NOT NULL DEFAULT 'high' CHECK (urgency IN ('high', 'low')),
    status             TEXT NOT NULL DEFAULT 'triggered' CHECK (status IN ('triggered', 'acknowledged', 'resolved')),
    team_handle        TEXT,
    policy_id          UUID REFERENCES on_call_escalation_policies(id) ON DELETE SET NULL,
    source             TEXT NOT NULL CHECK (source IN ('manual', 'incident', 'monitor')),
    incident_id        BIGINT REFERENCES incidents(id) ON DELETE SET NULL,
    monitor_id         BIGINT REFERENCES monitors(id) ON DELETE SET NULL,
    monitor_group      TEXT,
    -- Escalation position: the step notified next and the pass through the policy.
    step               INT NOT NULL DEFAULT 0,
    round              INT NOT NULL DEFAULT 0,
    next_escalation_at TIMESTAMPTZ DEFAULT now(),
    resolve_at         TIMESTAMPTZ,
    created_by         TEXT NOT NULL,
    created_by_id      UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    acknowledged_by    TEXT,
    acknowledged_at    TIMESTAMPTZ,
    resolved_by        TEXT,
    resolved_at        TIMESTAMPTZ
);
CREATE INDEX on_call_pages_org ON on_call_pages (org_id, created_at DESC);
CREATE INDEX on_call_pages_due ON on_call_pages (next_escalation_at) WHERE status = 'triggered';
CREATE INDEX on_call_pages_auto_resolve ON on_call_pages (resolve_at) WHERE status <> 'resolved';
CREATE INDEX on_call_pages_monitor ON on_call_pages (monitor_id) WHERE status <> 'resolved';

-- Everyone a page notified.
CREATE TABLE on_call_page_responders (
    page_id     BIGINT NOT NULL REFERENCES on_call_pages(id) ON DELETE CASCADE,
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    notified_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (page_id, user_id)
);
CREATE INDEX on_call_page_responders_user ON on_call_page_responders (user_id);

CREATE TABLE on_call_page_events (
    id         BIGSERIAL PRIMARY KEY,
    page_id    BIGINT NOT NULL REFERENCES on_call_pages(id) ON DELETE CASCADE,
    kind       TEXT NOT NULL CHECK (kind IN ('triggered', 'notified', 'escalated', 'acknowledged', 'resolved', 'reassigned', 'failed')),
    actor      TEXT NOT NULL,
    body       TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX on_call_page_events_page ON on_call_page_events (page_id, id);

-- How each member is reached: 'email' (their address) or channel handles ('slack-…', 'webhook-…'), per urgency.
CREATE TABLE on_call_profiles (
    org_id       TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    high_urgency TEXT[] NOT NULL DEFAULT '{email}',
    low_urgency  TEXT[] NOT NULL DEFAULT '{email}',
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, user_id)
);

-- Pages sent from an incident are on-call pages.
ALTER TABLE incident_pages ADD COLUMN oncall_page_id BIGINT REFERENCES on_call_pages(id) ON DELETE SET NULL;
