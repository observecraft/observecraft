-- Software Delivery → GitLab CI: follow every project the token sees (an admin's or auditor's token: the whole
-- instance), active in the last 30 days, instead of or besides a group and listed projects.
ALTER TABLE gitlab_ci_connections ADD COLUMN IF NOT EXISTS all_projects BOOLEAN NOT NULL DEFAULT false;
