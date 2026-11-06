#!/usr/bin/env bash
# Connects a fictional Slack workspace as account "tf-compat" for the application key's org. Workspaces
# are connected in the UI with a token Slack checks, so the row goes straight into Postgres (psql with
# OC_POSTGRES_URL, or $OC_TF_PSQL as run.sh uses it). teardown.sh removes it.
set -euo pipefail
org="$(curl -fsS "$TF_VAR_oc_api_url/api/v1/me" -H "Authorization: Bearer $TF_VAR_oc_app_key" | sed -E 's/.*"org":\{[^}]*"id":"([^"]+)".*/\1/')"
[ -n "$org" ] || { echo "could not read the org of the application key" >&2; exit 1; }
sql="INSERT INTO slack_accounts (org_id, name, team_id, team_name, bot_token_enc, installed_via) VALUES ('${org//\'/}', 'tf-compat', 'T0TFCOMPAT', 'Example Shop', 'unused', 'token') ON CONFLICT (org_id, name) DO NOTHING;"
if [ -n "${OC_TF_PSQL:-}" ]; then
  # shellcheck disable=SC2086 # a command line on purpose
  echo "$sql" | $OC_TF_PSQL -v ON_ERROR_STOP=1 -q
else
  psql "${OC_POSTGRES_URL:?the integration_slack_channel fixture needs OC_POSTGRES_URL or OC_TF_PSQL}" -v ON_ERROR_STOP=1 -q -c "$sql"
fi
echo "$org" >"$1/slack.org"
