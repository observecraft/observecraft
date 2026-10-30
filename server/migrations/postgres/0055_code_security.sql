-- Code Security (routes/code_security.rs): the libraries services load and repositories declare, the
-- findings of static scans, and the public advisories they are matched against.

-- Advisories (OSV.dev, the demo's own): public reference data shared by every org, so these two
-- tables have no org_id. They hold only what the advisory databases publish, never org data.
CREATE TABLE cs_advisories (
    id         TEXT PRIMARY KEY,                  -- GHSA-…, PYSEC-…, OCSA-DEMO-…
    aliases    TEXT[] NOT NULL DEFAULT '{}',      -- CVE-…
    summary    TEXT NOT NULL DEFAULT '',
    details    TEXT NOT NULL DEFAULT '',
    severity   TEXT NOT NULL DEFAULT 'unknown',   -- critical | high | medium | low | unknown
    score      REAL,                              -- CVSS base score
    vector     TEXT NOT NULL DEFAULT '',
    cwes       TEXT[] NOT NULL DEFAULT '{}',
    refs       JSONB NOT NULL DEFAULT '[]',       -- [{type, url}]
    published  TIMESTAMPTZ,
    modified   TIMESTAMPTZ,
    withdrawn  TIMESTAMPTZ,
    source     TEXT NOT NULL DEFAULT 'osv',       -- osv | demo
    fetched_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE cs_advisory_packages (
    advisory_id TEXT NOT NULL REFERENCES cs_advisories (id) ON DELETE CASCADE,
    ecosystem   TEXT NOT NULL,                    -- OSV ecosystem: Maven, npm, PyPI, NuGet, Go, RubyGems, Packagist, crates.io
    package     TEXT NOT NULL,
    ranges      JSONB NOT NULL DEFAULT '[]',      -- [{introduced, fixed, last_affected}] (ECOSYSTEM / SEMVER events)
    versions    TEXT[] NOT NULL DEFAULT '{}',     -- explicitly affected versions
    fixed       TEXT NOT NULL DEFAULT '',         -- the lowest fixed version, for remediation
    PRIMARY KEY (advisory_id, ecosystem, package)
);
CREATE INDEX cs_advisory_packages_package ON cs_advisory_packages (ecosystem, package);

-- Packages last looked up in OSV (the sync queries only packages some org uses).
CREATE TABLE cs_osv_packages (
    ecosystem  TEXT NOT NULL,
    package    TEXT NOT NULL,
    checked_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (ecosystem, package)
);

-- Libraries: loaded by a running service (tracer telemetry, source = runtime) or declared in a
-- repository (an SBOM upload, source = static).
CREATE TABLE cs_libraries (
    org_id     TEXT NOT NULL,
    source     TEXT NOT NULL,                     -- runtime | static
    ecosystem  TEXT NOT NULL,
    name       TEXT NOT NULL,
    version    TEXT NOT NULL,
    service    TEXT NOT NULL DEFAULT '',
    env        TEXT NOT NULL DEFAULT '',
    repository TEXT NOT NULL DEFAULT '',
    language   TEXT NOT NULL DEFAULT '',
    branch     TEXT NOT NULL DEFAULT '',
    commit_sha TEXT NOT NULL DEFAULT '',
    file       TEXT NOT NULL DEFAULT '',
    direct     BOOLEAN,
    dev        BOOLEAN NOT NULL DEFAULT false,
    licenses   TEXT[] NOT NULL DEFAULT '{}',
    first_seen TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, source, ecosystem, name, version, service, env, repository)
);
CREATE INDEX cs_libraries_package ON cs_libraries (ecosystem, name);

-- Static scan uploads (SBOM, SARIF): one row per upload.
CREATE TABLE cs_scans (
    org_id      TEXT NOT NULL,
    id          TEXT NOT NULL,
    kind        TEXT NOT NULL,                    -- sbom | sarif
    tool        TEXT NOT NULL DEFAULT '',
    repository  TEXT NOT NULL DEFAULT '',
    branch      TEXT NOT NULL DEFAULT '',
    commit_sha  TEXT NOT NULL DEFAULT '',
    pr_number   TEXT NOT NULL DEFAULT '',
    base_branch TEXT NOT NULL DEFAULT '',
    service     TEXT NOT NULL DEFAULT '',
    env         TEXT NOT NULL DEFAULT '',
    results     INTEGER NOT NULL DEFAULT 0,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, id)
);
CREATE INDEX cs_scans_received ON cs_scans (org_id, received_at DESC);

-- Rules of the static analyzers, from the SARIF reports that used them.
CREATE TABLE cs_rules (
    org_id      TEXT NOT NULL,
    kind        TEXT NOT NULL,                    -- sast | secret | iac
    tool        TEXT NOT NULL,
    rule_id     TEXT NOT NULL,
    name        TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    severity    TEXT NOT NULL DEFAULT 'unknown',
    cwes        TEXT[] NOT NULL DEFAULT '{}',
    help_uri    TEXT NOT NULL DEFAULT '',
    language    TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (org_id, tool, rule_id)
);

-- Findings of the static scans: code (SAST), secrets and infrastructure as code. A finding is
-- identified by rule, repository, file and its fingerprint (or line), so rescans update it.
CREATE TABLE cs_findings (
    org_id      TEXT NOT NULL,
    id          TEXT NOT NULL,
    kind        TEXT NOT NULL,                    -- sast | secret | iac
    tool        TEXT NOT NULL,
    rule_id     TEXT NOT NULL,
    severity    TEXT NOT NULL DEFAULT 'unknown',
    message     TEXT NOT NULL DEFAULT '',
    repository  TEXT NOT NULL DEFAULT '',
    branch      TEXT NOT NULL DEFAULT '',
    commit_sha  TEXT NOT NULL DEFAULT '',
    file        TEXT NOT NULL DEFAULT '',
    line        INTEGER NOT NULL DEFAULT 0,
    snippet     TEXT NOT NULL DEFAULT '',
    pr_number   TEXT NOT NULL DEFAULT '',
    scan_id     TEXT NOT NULL DEFAULT '',
    first_seen  TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen   TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at TIMESTAMPTZ,                      -- set when a later scan of the branch no longer reports it
    PRIMARY KEY (org_id, id)
);
CREATE INDEX cs_findings_kind ON cs_findings (org_id, kind, last_seen DESC);

-- Triage of a vulnerability (advisory × library) or a finding: muted, with a reason.
CREATE TABLE cs_triage (
    org_id     TEXT NOT NULL,
    key        TEXT NOT NULL,                     -- vuln:<advisory>:<ecosystem>:<package> | finding:<id> | iast:<hash>
    status     TEXT NOT NULL,                     -- open | muted
    reason     TEXT NOT NULL DEFAULT '',
    updated_by TEXT NOT NULL DEFAULT '',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, key)
);
