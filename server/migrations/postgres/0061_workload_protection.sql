-- Workload Protection (routes/workload_*.rs, datadog_api/csm_threats.rs): the agent policies and
-- agent rules the Datadog API and Terraform provider manage (`datadog_csm_threats_policy`,
-- `datadog_csm_threats_agent_rule`). The default policies and rules (our catalog,
-- `workload_catalog.rs`) are rows too, installed per org with `is_default`. Agent events are logs
-- (`source:runtime-security-agent`); detection rules are `security_rules` of type `workload_security`.

CREATE TABLE cws_policies (
    org_id          TEXT NOT NULL,
    -- Datadog's form: three groups of three lowercase letters and digits, e.g. `k3d-9pa-x2m`.
    id              TEXT NOT NULL,
    name            TEXT NOT NULL,
    description     TEXT NOT NULL DEFAULT '',
    enabled         BOOLEAN NOT NULL DEFAULT false,
    -- Inner lists are ANDed, the outer list ORed.
    host_tags_lists JSONB NOT NULL DEFAULT '[]',
    is_default      BOOLEAN NOT NULL DEFAULT false,
    pinned          BOOLEAN NOT NULL DEFAULT false,
    version         INTEGER NOT NULL DEFAULT 1,
    updated_by      TEXT NOT NULL DEFAULT '',
    updated_by_name TEXT NOT NULL DEFAULT '',
    updated_by_handle TEXT NOT NULL DEFAULT '',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, id)
);
CREATE UNIQUE INDEX cws_policies_name ON cws_policies (org_id, name);

CREATE TABLE cws_agent_rules (
    org_id          TEXT NOT NULL,
    id              TEXT NOT NULL,
    name            TEXT NOT NULL,
    policy_id       TEXT NOT NULL DEFAULT '',
    description     TEXT NOT NULL DEFAULT '',
    expression      TEXT NOT NULL,
    enabled         BOOLEAN NOT NULL DEFAULT true,
    silent          BOOLEAN,
    blocking        BOOLEAN NOT NULL DEFAULT false,
    product_tags    TEXT[] NOT NULL DEFAULT '{}',
    actions         JSONB NOT NULL DEFAULT '[]',
    agent_version   TEXT NOT NULL DEFAULT '',
    is_default      BOOLEAN NOT NULL DEFAULT false,
    created_by_name TEXT NOT NULL DEFAULT '',
    created_by_handle TEXT NOT NULL DEFAULT '',
    updated_by_name TEXT NOT NULL DEFAULT '',
    updated_by_handle TEXT NOT NULL DEFAULT '',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, id)
);
-- A rule name is unique within its policy (the Agent identifies rules by name).
CREATE UNIQUE INDEX cws_agent_rules_name ON cws_agent_rules (org_id, policy_id, name);
