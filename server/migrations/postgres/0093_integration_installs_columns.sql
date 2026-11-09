-- 0001 made an older integration_installs (integration_id, settings) that nothing used; 0092's CREATE TABLE IF NOT
-- EXISTS kept it, so the server's queries on `integration` failed ("Query failed" on Integrations). Give the table
-- 0092's columns, keeping any rows.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns WHERE table_schema = current_schema() AND table_name = 'integration_installs' AND column_name = 'integration_id') THEN
        ALTER TABLE integration_installs RENAME COLUMN integration_id TO integration;
    END IF;
END $$;
ALTER TABLE integration_installs DROP COLUMN IF EXISTS settings;
ALTER TABLE integration_installs ADD COLUMN IF NOT EXISTS installed_by UUID REFERENCES users(id) ON DELETE SET NULL;
