-- Browser tests (Synthetic Monitoring): steps in Datadog's browser step model, replayed by oc-browser after the
-- start URL (request.url). Their screenshots and videos are in ClickHouse (otel.synthetics_artifacts).
ALTER TABLE synthetic_tests DROP CONSTRAINT IF EXISTS synthetic_tests_type_check;
ALTER TABLE synthetic_tests ADD CONSTRAINT synthetic_tests_type_check CHECK (type IN ('http', 'ssl', 'dns', 'tcp', 'browser'));
ALTER TABLE synthetic_tests ADD COLUMN IF NOT EXISTS steps JSONB NOT NULL DEFAULT '[]';
