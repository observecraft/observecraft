-- Real User Monitoring applications (Digital Experience → RUM → Manage Applications), managed through
-- Datadog's `/api/v2/rum/applications` (the Terraform provider's `datadog_rum_application`) and the UI.
-- An application's client token is public by design: browser and mobile SDKs embed it to send events to
-- the intake (`POST /api/v2/rum`), which accepts it only there.
CREATE TABLE rum_applications (
    id                                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id                            TEXT NOT NULL REFERENCES orgs(id) ON DELETE CASCADE,
    name                              TEXT NOT NULL,
    type                              TEXT NOT NULL CHECK (type IN ('browser', 'ios', 'android', 'react-native', 'flutter', 'roku', 'electron', 'unity', 'kotlin-multiplatform')),
    client_token                      TEXT NOT NULL UNIQUE,
    tags                              TEXT[] NOT NULL DEFAULT '{}',
    -- Which events are kept: all of them, only sessions with errors, or none (collection off).
    rum_event_processing_state        TEXT NOT NULL DEFAULT 'ALL' CHECK (rum_event_processing_state IN ('ALL', 'ERROR_FOCUSED_MODE', 'NONE')),
    rum_event_processing_modified_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    product_analytics_retention_state TEXT NOT NULL DEFAULT 'NONE' CHECK (product_analytics_retention_state IN ('MAX', 'NONE')),
    product_analytics_modified_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_by                        UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_by                        UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at                        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at                        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX rum_applications_org ON rum_applications (org_id, lower(name));
