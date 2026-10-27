-- Per-organization metric retention (docs/adr/0001-metrics-storage.md): days at full resolution and
-- as 5-minute and 1-hour rollups, and the limits the instance admin sets. NULL = the install's
-- default (OC_RETENTION_METRICS_*); the API only accepts the fixed tiers.
ALTER TABLE org_data_settings
    ADD COLUMN metrics_retention_days     INTEGER CHECK (metrics_retention_days BETWEEN 1 AND 3650),
    ADD COLUMN metrics_5m_days            INTEGER CHECK (metrics_5m_days BETWEEN 1 AND 3650),
    ADD COLUMN metrics_1h_days            INTEGER CHECK (metrics_1h_days BETWEEN 1 AND 3650),
    ADD COLUMN max_metrics_retention_days INTEGER CHECK (max_metrics_retention_days BETWEEN 1 AND 3650),
    ADD COLUMN max_metrics_5m_days        INTEGER CHECK (max_metrics_5m_days BETWEEN 1 AND 3650),
    ADD COLUMN max_metrics_1h_days        INTEGER CHECK (max_metrics_1h_days BETWEEN 1 AND 3650);
