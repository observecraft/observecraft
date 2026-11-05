-- Per-org ingest limits (instance admins, `PATCH /api/v1/instance/orgs/{id}/limits`): MiB and requests per second the
-- intake accepts from the org's keys. NULL follows the install (OC_INTAKE_ORG_MBPS / OC_INTAKE_ORG_RPS, unlimited by
-- default).
ALTER TABLE org_data_settings
    ADD COLUMN IF NOT EXISTS ingest_mbps DOUBLE PRECISION,
    ADD COLUMN IF NOT EXISTS ingest_rps DOUBLE PRECISION;
