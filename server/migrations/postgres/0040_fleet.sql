-- Fleet Automation: what the Datadog Agent reports about itself, next to its host metadata.
-- Every check instance with its status (`agent_checks` metadata, every 10 minutes): Fleet View
-- shows the failing ones. `[{name, instance, status, message}]`.
ALTER TABLE datadog_hosts ADD COLUMN agent_check_runs JSONB NOT NULL DEFAULT '[]';
-- When the checks were last reported: the Agent's most frequent metadata, so its liveness.
ALTER TABLE datadog_hosts ADD COLUMN checks_at TIMESTAMPTZ;
-- The Agent's inventory (`/api/v1/metadata` `agent_metadata`): enabled features, configuration
-- sources, install method and, when the Agent sends it, its scrubbed configuration. The API
-- redacts secrets again before showing it.
ALTER TABLE datadog_hosts ADD COLUMN agent_inventory JSONB;
