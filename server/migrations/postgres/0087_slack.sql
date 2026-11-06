-- Slack integration: workspaces connected through a Slack app (bot token), the channels monitors
-- notify (`@slack-<account>-<channel>`, Datadog's `datadog_integration_slack_channel`), and the
-- OAuth installs in progress.

CREATE TABLE slack_accounts (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id             TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    -- Datadog's account name (`/api/v1/integration/slack/configuration/accounts/{account_name}`) and the
    -- middle part of `@slack-<account>-<channel>`.
    name               TEXT NOT NULL,
    team_id            TEXT NOT NULL,
    team_name          TEXT NOT NULL DEFAULT '',
    bot_user_id        TEXT NOT NULL DEFAULT '',
    -- AES-256-GCM(OC_SECRET_KEY): the bot token (xoxb-…).
    bot_token_enc      TEXT NOT NULL,
    -- AES-256-GCM(OC_SECRET_KEY): the signing secret of the workspace's own Slack app (manual installs),
    -- for its interactive buttons; installs through this server's app use OC_SLACK_SIGNING_SECRET.
    signing_secret_enc TEXT,
    scopes             TEXT[] NOT NULL DEFAULT '{}',
    -- oauth | token
    installed_via      TEXT NOT NULL CHECK (installed_via IN ('oauth', 'token')),
    created_by         UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (org_id, name)
);
CREATE INDEX slack_accounts_team ON slack_accounts (team_id);

CREATE TABLE slack_channels (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    account_id  UUID NOT NULL REFERENCES slack_accounts(id) ON DELETE CASCADE,
    -- As configured, usually with its '#': `#checkout-alerts`.
    name        TEXT NOT NULL,
    -- Slack's conversation id, resolved on first delivery.
    channel_id  TEXT,
    -- Datadog's display options: message, notified, snapshot, tags, mute_buttons.
    display     JSONB NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX slack_channels_name ON slack_channels (account_id, lower(ltrim(name, '#')));
CREATE INDEX slack_channels_org ON slack_channels (org_id);

-- OAuth installs started from the Slack integration page (single use, 10 minutes).
CREATE TABLE slack_oauth_states (
    state       TEXT PRIMARY KEY,
    org_id      TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
