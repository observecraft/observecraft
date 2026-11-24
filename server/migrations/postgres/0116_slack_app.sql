-- The ObserveCraft Slack app (ADR-0010): commands, events and actions from Slack.

-- The organization a Slack user acts in, when their workspace is connected to several they are a member of
-- (through that organization's connection of the workspace).
CREATE TABLE slack_user_orgs (
    team_id       TEXT NOT NULL,
    slack_user_id TEXT NOT NULL,
    account_id    UUID NOT NULL REFERENCES slack_accounts(id) ON DELETE CASCADE,
    chosen_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (team_id, slack_user_id)
);

-- Events API deliveries already handled: Slack retries an event until it gets a 200 in time. Kept a day.
CREATE TABLE slack_events (
    event_id    TEXT PRIMARY KEY,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX slack_events_received ON slack_events (received_at);
