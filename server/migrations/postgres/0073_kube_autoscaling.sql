-- Kubernetes → Autoscaling. `kube_autoscaling_settings`: the org's prices per vCPU-hour and GiB-hour of
-- requests, which idle costs and projected savings are computed with (defaults when there is no row).
-- `kube_autoscaling_profiles`: DatadogPodAutoscalerClusterProfile templates saved in ObserveCraft (Create
-- Profile); users apply their manifests to clusters, where the Cluster Agent's orchestrator check reports
-- them back. Names are unique per org, like cluster-scoped profiles in a cluster.
CREATE TABLE IF NOT EXISTS kube_autoscaling_settings (
    org_id                TEXT PRIMARY KEY,
    cpu_hour_price        DOUBLE PRECISION NOT NULL,
    memory_gib_hour_price DOUBLE PRECISION NOT NULL,
    modified_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS kube_autoscaling_profiles (
    org_id      TEXT NOT NULL,
    id          UUID NOT NULL DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL,
    template    JSONB NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    modified_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, id),
    UNIQUE (org_id, name)
);
