-- Datadog-compatible management API (ADR-0003): the Datadog documents of SLOs and SLO corrections as
-- Datadog's clients sent them, next to the rows they were translated into (routes/datadog_api/slos.rs,
-- slo_corrections.rs). Reads return the document with the server-owned fields filled in; fields the
-- rows hold are taken from the rows when they were changed elsewhere (e.g. in the UI).

CREATE TABLE datadog_slo_documents (
    org_id     TEXT NOT NULL,
    slo_id     UUID NOT NULL,
    -- The ServiceLevelObjectiveRequest as received, without server-owned fields.
    document   JSONB NOT NULL,
    PRIMARY KEY (org_id, slo_id),
    FOREIGN KEY (org_id, slo_id) REFERENCES slos(org_id, id) ON DELETE CASCADE
);

CREATE TABLE datadog_slo_correction_documents (
    correction_id UUID PRIMARY KEY REFERENCES slo_corrections(id) ON DELETE CASCADE,
    org_id        TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    -- The correction's `attributes` as received.
    attributes    JSONB NOT NULL,
    modified_by   UUID REFERENCES users(id) ON DELETE SET NULL,
    modified_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX datadog_slo_correction_documents_org ON datadog_slo_correction_documents (org_id);
