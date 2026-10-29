-- Datadog-compatible dashboards and dashboard lists (docs/adr/0003-datadog-compatible-management-api.md,
-- src/routes/datadog_api/dashboards.rs and dashboard_lists.rs).
--
-- A dashboard written through /api/v1/dashboard keeps the Datadog document as received (widget ids
-- assigned, server-owned fields removed): reads return it, so Terraform sees what it sent. The UI's
-- document (`definition`) is derived from it on write; edits made in the UI are folded back into the
-- Datadog view on read. NULL for dashboards never written through the Datadog API.
ALTER TABLE dashboards ADD COLUMN datadog_document JSONB;

-- Datadog's dashboard lists have integer ids, an author and timestamps.
ALTER TABLE dashboard_lists ADD COLUMN datadog_id BIGSERIAL;
CREATE UNIQUE INDEX dashboard_lists_datadog_id ON dashboard_lists (datadog_id);
ALTER TABLE dashboard_lists ADD COLUMN author_id UUID REFERENCES users(id) ON DELETE SET NULL;
ALTER TABLE dashboard_lists ADD COLUMN created_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE dashboard_lists ADD COLUMN modified_at TIMESTAMPTZ NOT NULL DEFAULT now();
