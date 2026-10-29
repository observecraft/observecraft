-- Datadog-compatible monitors and downtimes (docs/adr/0003-datadog-compatible-management-api.md,
-- src/routes/datadog_api/{monitors,downtimes}.rs).
--
-- `datadog_document` keeps the Datadog document as it was last written through the compatible API
-- (server-owned fields left out), so reads return what Terraform sent; the monitor or downtime row
-- stays the source of truth for everything we evaluate, and reads put its values back into the
-- document where it was changed elsewhere (our UI). NULL for rows never written through it.

ALTER TABLE monitors ADD COLUMN datadog_document JSONB;

-- `uuid`: the downtime's id in Datadog's v2 API (v1 uses `id`).
-- `weekdays`: a daily recurring downtime limited to these days (0 = Monday … 6 = Sunday, UTC),
--   Datadog's weekly recurrences on several days.
-- `datadog_version`: which API wrote `datadog_document` ('v1' or 'v2').
ALTER TABLE downtimes
    ADD COLUMN uuid             UUID NOT NULL DEFAULT gen_random_uuid(),
    ADD COLUMN weekdays         SMALLINT[] NOT NULL DEFAULT '{}' CHECK (weekdays <@ '{0,1,2,3,4,5,6}'),
    ADD COLUMN datadog_document JSONB,
    ADD COLUMN datadog_version  TEXT CHECK (datadog_version IN ('v1', 'v2')),
    ADD COLUMN modified_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    ADD COLUMN modified_by      UUID REFERENCES users(id) ON DELETE SET NULL;
CREATE UNIQUE INDEX downtimes_uuid ON downtimes (uuid);
ALTER TABLE downtimes ADD CONSTRAINT downtimes_weekdays_daily CHECK (cardinality(weekdays) = 0 OR recurrence = 'daily');
