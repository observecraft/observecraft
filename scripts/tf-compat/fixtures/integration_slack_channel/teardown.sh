#!/usr/bin/env bash
# Removes the workspace setup.sh connected (and with it any channel left behind).
set -euo pipefail
dir="$1"
[ -f "$dir/slack.org" ] || exit 0
org="$(cat "$dir/slack.org")"
sql="DELETE FROM slack_accounts WHERE org_id = '${org//\'/}' AND name = 'tf-compat';"
if [ -n "${OC_TF_PSQL:-}" ]; then
  # shellcheck disable=SC2086 # a command line on purpose
  echo "$sql" | $OC_TF_PSQL -v ON_ERROR_STOP=1 -q
else
  psql "$OC_POSTGRES_URL" -v ON_ERROR_STOP=1 -q -c "$sql"
fi
rm -f "$dir/slack.org"
