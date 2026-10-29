-- Organizations and ingest keys for the load profile (`oc-tracegen load`, docs/adr/0002). Local only.
--
--   psql "$OC_POSTGRES_URL" -v orgs=4 -f scripts/load-orgs.sql
--
-- Creates `load-org-1` … `load-org-N` with the ingest keys `ocik_load_org_<n>_local_only` (the
-- profile's default OC_LOAD_KEYS). Keys are stored hashed (SHA-256), as the API stores them. Run it
-- after the server has applied its migrations; running it again changes nothing.
\set ON_ERROR_STOP on
\if :{?orgs}
\else
  \set orgs 4
\endif

INSERT INTO orgs (id, name)
SELECT 'load-org-' || n, 'Load test org ' || n
FROM generate_series(1, :orgs) AS n
ON CONFLICT (id) DO NOTHING;

INSERT INTO ingest_keys (org_id, name, key_hash, prefix)
SELECT 'load-org-' || n, 'Load profile', sha256(convert_to('ocik_load_org_' || n || '_local_only', 'UTF8')), left('ocik_load_org_' || n || '_local_only', 10)
FROM generate_series(1, :orgs) AS n
ON CONFLICT (key_hash) DO NOTHING;

SELECT k.org_id, k.prefix, k.created_at FROM ingest_keys k WHERE k.org_id LIKE 'load-org-%' AND k.name = 'Load profile' ORDER BY k.org_id;
