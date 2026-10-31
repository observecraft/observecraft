-- Cloud Security: the software bills of materials the Datadog Agent sends for hosts and container
-- images (`/api/v2/sbom`), matched against the advisories Code Security keeps (routes/cloud_security.rs).
-- Misconfigurations are evaluated on the resource catalog when read; their triage is Code Security's
-- (`cs_triage`, keys `misconfig:…` and `csm-vuln:…`).

-- What an SBOM describes: a host (its file system) or a container image.
CREATE TABLE csm_sbom_entities (
    org_id       TEXT NOT NULL,
    id           TEXT NOT NULL,                     -- host name, or image id (sha256:…)
    kind         TEXT NOT NULL,                     -- host | container_image
    name         TEXT NOT NULL,                     -- host name, or the image's first repo tag
    host         TEXT NOT NULL DEFAULT '',          -- the host that scanned it
    repo_tags    TEXT[] NOT NULL DEFAULT '{}',
    repo_digests TEXT[] NOT NULL DEFAULT '{}',
    in_use       BOOLEAN NOT NULL DEFAULT false,    -- a running container uses the image (hosts: always)
    os           TEXT NOT NULL DEFAULT '',          -- distribution and version (debian 12)
    kernel       TEXT NOT NULL DEFAULT '',
    arch         TEXT NOT NULL DEFAULT '',
    env          TEXT NOT NULL DEFAULT '',
    tags         TEXT[] NOT NULL DEFAULT '{}',
    first_seen   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, id)
);

-- The packages of an SBOM (OS packages and language libraries), by OSV ecosystem.
CREATE TABLE csm_packages (
    org_id     TEXT NOT NULL,
    entity_id  TEXT NOT NULL,
    ecosystem  TEXT NOT NULL,                       -- Debian:12, Ubuntu:22.04:LTS, Alpine:v3.19, Maven, npm, …
    name       TEXT NOT NULL,
    version    TEXT NOT NULL,
    purl       TEXT NOT NULL DEFAULT '',
    first_seen TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, entity_id, ecosystem, name, version)
);
CREATE INDEX csm_packages_package ON csm_packages (ecosystem, name);
