-- Archived logs loaded back for searching (Logs → Rehydrations). Same columns as oc.logs plus the
-- rehydration they belong to; kept until that rehydration expires (no age-based TTL: the logs are old).
CREATE TABLE IF NOT EXISTS oc.logs_rehydrated AS oc.logs;
ALTER TABLE oc.logs_rehydrated REMOVE TTL;
ALTER TABLE oc.logs_rehydrated ADD COLUMN IF NOT EXISTS rehydration_id String DEFAULT '';
