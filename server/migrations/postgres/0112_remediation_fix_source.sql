-- Remediation, level Fix in infrastructure (ADR-0008): the resource an infrastructure fix changes and the source it
-- was changed in (its provenance: repository, path, address or Argo CD Application, confidence and how it was found;
-- the Terraform resource located; whether an inferred source was confirmed), named in the merge request. Fixes from
-- Tag Coverage ("Add tags with Assist") are finding_kind 'resource' with the directory tags were added under.
-- method gains 'editor' (a deterministic edit: an S3 public access block, the AWS provider's default_tags, OCI labels).
-- assist_runs gains the kinds 'fix_infra' and 'add_tags'. See routes/remediation.rs and routes/remediation_infra.rs.
ALTER TABLE remediation_fixes ADD COLUMN IF NOT EXISTS source JSONB;
