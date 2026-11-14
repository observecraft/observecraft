-- Code Security from GitLab: the GitLab CI connection can also import GitLab's own scanners' results (its
-- Vulnerability Report: SAST, Secret Detection, Dependency Scanning), polled by the server (`crate::gitlab::security`,
-- `cloud_crawl_jobs` provider `gitlab-security`, one job per project).
ALTER TABLE gitlab_ci_connections ADD COLUMN IF NOT EXISTS import_code_security BOOLEAN NOT NULL DEFAULT false;

-- Where a static finding comes from: an upload (datadog-ci sarif upload) or GitLab's Vulnerability Report. GitLab's
-- findings keep GitLab's vulnerability id, link, state and dismissal reason; a re-import updates them in place.
ALTER TABLE cs_findings ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'upload';        -- upload | gitlab
ALTER TABLE cs_findings ADD COLUMN IF NOT EXISTS external_id TEXT NOT NULL DEFAULT '';         -- GitLab's vulnerability id
ALTER TABLE cs_findings ADD COLUMN IF NOT EXISTS url TEXT NOT NULL DEFAULT '';                 -- GitLab's vulnerability page
ALTER TABLE cs_findings ADD COLUMN IF NOT EXISTS upstream_state TEXT NOT NULL DEFAULT '';      -- detected | confirmed | dismissed | resolved
ALTER TABLE cs_findings ADD COLUMN IF NOT EXISTS upstream_reason TEXT NOT NULL DEFAULT '';     -- GitLab's dismissal reason
ALTER TABLE cs_findings ADD COLUMN IF NOT EXISTS identifiers TEXT[] NOT NULL DEFAULT '{}';     -- rule ids, CWE, CVE of the finding
CREATE INDEX IF NOT EXISTS cs_findings_source ON cs_findings (org_id, source, repository);

-- Vulnerable libraries a source control's dependency scanner reported (GitLab's Dependency Scanning), with the
-- scanner's own advisory data; shown with (and merged into) the OSV-matched library vulnerabilities.
CREATE TABLE IF NOT EXISTS cs_dependency_findings (
    org_id          TEXT NOT NULL,
    id              TEXT NOT NULL,
    source          TEXT NOT NULL DEFAULT 'gitlab',
    external_id     TEXT NOT NULL DEFAULT '',
    tool            TEXT NOT NULL DEFAULT '',
    repository      TEXT NOT NULL DEFAULT '',
    branch          TEXT NOT NULL DEFAULT '',
    ecosystem       TEXT NOT NULL DEFAULT '',          -- OSV ecosystem, from the lockfile ('' when unknown)
    package         TEXT NOT NULL,
    version         TEXT NOT NULL DEFAULT '',
    file            TEXT NOT NULL DEFAULT '',
    advisory_id     TEXT NOT NULL,                     -- CVE-… when the scanner names one, else its own id
    aliases         TEXT[] NOT NULL DEFAULT '{}',
    title           TEXT NOT NULL DEFAULT '',
    description     TEXT NOT NULL DEFAULT '',
    severity        TEXT NOT NULL DEFAULT 'unknown',
    cwes            TEXT[] NOT NULL DEFAULT '{}',
    solution        TEXT NOT NULL DEFAULT '',
    fixed           TEXT NOT NULL DEFAULT '',
    url             TEXT NOT NULL DEFAULT '',
    upstream_state  TEXT NOT NULL DEFAULT '',
    upstream_reason TEXT NOT NULL DEFAULT '',
    first_seen      TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen       TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at     TIMESTAMPTZ,
    PRIMARY KEY (org_id, id)
);
CREATE INDEX IF NOT EXISTS cs_dependency_findings_repo ON cs_dependency_findings (org_id, source, repository);
