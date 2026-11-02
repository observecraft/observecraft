-- The last answer of ObserveCraft's external recommender to each DatadogPodAutoscaler target the Cluster Agent
-- asks about (every 30 seconds per autoscaler): the Autoscaling pages show the replicas it recommended and why,
-- or the error the Cluster Agent got. One row per (org, cluster, namespace, kind, name), overwritten.
CREATE TABLE IF NOT EXISTS kube_autoscaler_recommendations (
    org_id           TEXT NOT NULL,
    cluster          TEXT NOT NULL,
    namespace        TEXT NOT NULL,
    kind             TEXT NOT NULL,
    name             TEXT NOT NULL,
    current_replicas INTEGER NOT NULL,
    target_replicas  INTEGER NOT NULL,
    reason           TEXT NOT NULL,
    error            TEXT,
    observed         JSONB NOT NULL DEFAULT '[]',
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, cluster, namespace, kind, name)
);
