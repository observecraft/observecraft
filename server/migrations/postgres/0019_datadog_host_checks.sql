-- The checks the Datadog Agent runs on each host (its `agent_checks` metadata, every 10 minutes):
-- the host list's Apps column.
ALTER TABLE datadog_hosts ADD COLUMN agent_checks TEXT[] NOT NULL DEFAULT '{}';
