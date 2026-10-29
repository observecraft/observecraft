#!/usr/bin/env bash
# End-to-end check of the Datadog-compatible management API with Datadog's own Terraform provider
# (docs/adr/0003-datadog-compatible-management-api.md), run by OpenTofu (MPL-2.0).
#
# For every fixture directory under scripts/tf-compat/fixtures/<resource>/ and for both ways the
# provider authenticates (api_key + app_key, and bearer_token), it runs:
#   init → apply → plan (must show no changes) → [apply + plan of the update/ overlay] → destroy
#
# A fixture may have a setup.sh, run with the run's working directory before init (it can create
# what the fixture needs through our own API and write *.auto.tfvars there; TF_VAR_oc_api_url and
# TF_VAR_oc_app_key are set), and a teardown.sh, run with the same directory after destroy.
#
# Usage:
#   scripts/tf-compat/run.sh [--local] [fixture ...]
#
# Against a running server (the default):
#   OC_TF_API_URL   server URL, e.g. http://localhost:8080
#   OC_TF_APP_KEY   application key (ocak_…) of a user with the Standard or Admin role
#   OC_TF_API_KEY   ingest key (ocik_…) of the same org; without it only the bearer_token run happens
#   (the application_key fixture needs OC_DATADOG_API_ALLOW_APP_KEY_CREATION=true on that server)
#
# --local builds and starts observecraft-server itself on 127.0.0.1:$OC_TF_PORT (default 18480,
# internal listener on the next port) against OC_POSTGRES_URL, OC_CLICKHOUSE_URL and OC_CACHE_URL,
# creates an application key for the default org's first admin and an ingest key with psql (or
# $OC_TF_PSQL, e.g. "docker exec -i my-postgres psql -U oc -d mydb"), and stops the server at the end.
# Its log goes to $OC_TF_WORK/server.log.
#
# Other settings: OC_TF_CACHE (OpenTofu binary and provider plugins, default ~/.cache/observecraft/tf-compat),
# OC_TF_WORK (per-run working directories, default scripts/tf-compat/.work, git-ignored).
set -euo pipefail

TOFU_VERSION=1.13.2
PROVIDER_VERSION=4.27.0
# From https://github.com/opentofu/opentofu/releases/download/v1.13.2/tofu_1.13.2_SHA256SUMS
checksum() {
  case "$1" in
    darwin_arm64) echo cb46112c095f1c4da721b6a18c8368f67341896a62205d1c117299e85993af61 ;;
    darwin_amd64) echo 2093d0f881d911eb133cf51d2d7e09f22da71b1e6465c60826a314d217b0b84e ;;
    linux_amd64) echo 723bebb0df8411daff1e5fc3862e28ff5581fe5634087f4c29a9aa41d78208b5 ;;
    linux_arm64) echo 6e815c58a09ee957de606ff48cefad082942e70afc43b737ff975654c54ec453 ;;
    *) return 1 ;;
  esac
}

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
cache="${OC_TF_CACHE:-$HOME/.cache/observecraft/tf-compat}"
work="${OC_TF_WORK:-$here/.work}"
local_server=false
fixtures=()
for arg in "$@"; do
  case "$arg" in
    --local) local_server=true ;;
    -h | --help) sed -n '2,29p' "$0"; exit 0 ;;
    *) fixtures+=("$arg") ;;
  esac
done
if [ ${#fixtures[@]} -eq 0 ]; then
  for d in "$here"/fixtures/*/; do fixtures+=("$(basename "$d")"); done
fi

log() { printf '\033[1m[tf-compat]\033[0m %s\n' "$*" >&2; }
die() { log "error: $*"; exit 1; }

sha256() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }

# ---------------------------------------------------------------- OpenTofu
install_tofu() {
  local os arch platform zip dir
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m)"
  case "$arch" in x86_64 | amd64) arch=amd64 ;; arm64 | aarch64) arch=arm64 ;; *) die "unsupported CPU $arch" ;; esac
  platform="${os}_${arch}"
  dir="$cache/tofu-$TOFU_VERSION-$platform"
  tofu="$dir/tofu"
  [ -x "$tofu" ] && return
  local want
  want="$(checksum "$platform")" || die "no OpenTofu checksum for $platform"
  mkdir -p "$dir"
  zip="$dir/tofu.zip"
  log "downloading OpenTofu $TOFU_VERSION ($platform)"
  curl -fsSL -o "$zip" "https://github.com/opentofu/opentofu/releases/download/v$TOFU_VERSION/tofu_${TOFU_VERSION}_$platform.zip"
  [ "$(sha256 "$zip")" = "$want" ] || { rm -f "$zip"; die "OpenTofu checksum mismatch"; }
  unzip -q -o "$zip" tofu -d "$dir"
  rm -f "$zip"
}

# ---------------------------------------------------------------- local server
server_pid=""
stop_server() {
  if [ -n "$server_pid" ]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
}

psql_run() {
  if [ -n "${OC_TF_PSQL:-}" ]; then
    # shellcheck disable=SC2086 # a command line on purpose
    $OC_TF_PSQL -v ON_ERROR_STOP=1 -q "$@"
  else
    psql "$OC_POSTGRES_URL" -v ON_ERROR_STOP=1 -q "$@"
  fi
}

start_local_server() {
  : "${OC_POSTGRES_URL:?--local needs OC_POSTGRES_URL}" "${OC_CLICKHOUSE_URL:?--local needs OC_CLICKHOUSE_URL}" "${OC_CACHE_URL:?--local needs OC_CACHE_URL}"
  local port="${OC_TF_PORT:-18480}"
  log "building observecraft-server"
  (cd "$repo/server" && cargo build -q --bin observecraft-server)
  log "starting observecraft-server on 127.0.0.1:$port (log: $work/server.log)"
  # The application_key fixture creates keys with the provider's own key (off by default on servers).
  OC_HTTP_ADDR="127.0.0.1:$port" OC_LOGPROC_ADDR="127.0.0.1:$((port + 1))" OC_SEED_DEMO=false OC_DATADOG_API_ALLOW_APP_KEY_CREATION=true \
    "$repo/server/target/debug/observecraft-server" >"$work/server.log" 2>&1 &
  server_pid=$!
  echo "$server_pid" >"$work/server.pid"
  for _ in $(seq 1 300); do
    curl -fs "http://127.0.0.1:$port/healthz" >/dev/null 2>&1 && break
    kill -0 "$server_pid" 2>/dev/null || die "the server exited, see $work/server.log"
    sleep 1
  done
  curl -fs "http://127.0.0.1:$port/healthz" >/dev/null || die "the server did not start, see $work/server.log"
  OC_TF_API_URL="http://127.0.0.1:$port"
  OC_TF_APP_KEY="ocak_tfcompat_$(openssl rand -hex 24)"
  OC_TF_API_KEY="ocik_tfcompat_$(openssl rand -hex 24)"
  # Keys are stored as SHA-256; the org's first admin acts through the application key.
  psql_run -v app_key="$OC_TF_APP_KEY" -v api_key="$OC_TF_API_KEY" <<'SQL'
INSERT INTO app_keys (org_id, user_id, name, key_hash, prefix)
SELECT u.org_id, u.id, 'tf-compat', sha256(convert_to(:'app_key', 'UTF8')), left(:'app_key', 12)
FROM users u JOIN user_roles r ON r.user_id = u.id AND r.role_id = u.org_id || '-admin'
WHERE u.org_id = 'default' ORDER BY u.created_at LIMIT 1;
INSERT INTO ingest_keys (org_id, name, key_hash, prefix)
VALUES ('default', 'tf-compat', sha256(convert_to(:'api_key', 'UTF8')), left(:'api_key', 12));
SQL
}

# ---------------------------------------------------------------- runs
provider_tf() {
  local auth="$1"
  cat <<HCL
terraform {
  required_providers {
    datadog = {
      source  = "DataDog/datadog"
      version = "= $PROVIDER_VERSION"
    }
  }
}

variable "oc_api_url" { type = string }
variable "oc_api_key" {
  type      = string
  sensitive = true
  default   = ""
}
variable "oc_app_key" {
  type      = string
  sensitive = true
}

HCL
  if [ "$auth" = keys ]; then
    cat <<'HCL'
provider "datadog" {
  api_url  = var.oc_api_url
  api_key  = var.oc_api_key
  app_key  = var.oc_app_key
  validate = true
}
HCL
  else
    cat <<'HCL'
provider "datadog" {
  api_url      = var.oc_api_url
  bearer_token = var.oc_app_key
  validate     = true
}
HCL
  fi
}

step() {
  local name="$1"
  shift
  log "  $name"
  "$@" >>"$dir/run.log" 2>&1
}

# Runs one fixture with one way of authenticating; returns non-zero on the first failing step.
run_fixture() {
  local fixture="$1" auth="$2"
  dir="$work/$fixture-$auth"
  rm -rf "$dir"
  mkdir -p "$dir"
  find "$here/fixtures/$fixture" -maxdepth 1 -type f -name '*.tf' -exec cp {} "$dir/" \;
  provider_tf "$auth" >"$dir/provider.tf"
  cd "$dir"
  log "$fixture ($auth)"
  if [ -f "$here/fixtures/$fixture/setup.sh" ]; then step setup bash "$here/fixtures/$fixture/setup.sh" "$dir" || return 1; fi
  step init "$tofu" init -input=false -no-color || return 1
  step apply "$tofu" apply -auto-approve -input=false -no-color || return 1
  step "plan shows no changes" "$tofu" plan -detailed-exitcode -input=false -no-color || return 1
  if [ -d "$here/fixtures/$fixture/update" ]; then
    find "$here/fixtures/$fixture/update" -maxdepth 1 -type f -name '*.tf' -exec cp {} "$dir/" \;
    step "apply update" "$tofu" apply -auto-approve -input=false -no-color || return 1
    step "plan after update shows no changes" "$tofu" plan -detailed-exitcode -input=false -no-color || return 1
  fi
  step destroy "$tofu" destroy -auto-approve -input=false -no-color || return 1
  teardown_fixture "$fixture"
}

teardown_fixture() {
  if [ -f "$here/fixtures/$1/teardown.sh" ]; then step teardown bash "$here/fixtures/$1/teardown.sh" "$dir"; fi
}

# The provider must refuse to start with a wrong application key (validation really happens).
check_wrong_key_rejected() {
  local auth="$1"
  dir="$work/wrong-key-$auth"
  rm -rf "$dir"
  mkdir -p "$dir"
  cp "$here/fixtures/provider/main.tf" "$dir/"
  provider_tf "$auth" >"$dir/provider.tf"
  cd "$dir"
  log "a wrong application key is rejected ($auth)"
  step init "$tofu" init -input=false -no-color || return 1
  if TF_VAR_oc_app_key="ocak_wrong_$(openssl rand -hex 8)" "$tofu" plan -input=false -no-color >>"$dir/run.log" 2>&1; then
    log "  the provider accepted a wrong key"
    return 1
  fi
  grep -q "403" "$dir/run.log" || { log "  expected a 403 from the server"; return 1; }
}

mkdir -p "$cache/plugins" "$work"
install_tofu
if $local_server; then
  trap stop_server EXIT
  start_local_server
fi
: "${OC_TF_API_URL:?set OC_TF_API_URL (or pass --local)}" "${OC_TF_APP_KEY:?set OC_TF_APP_KEY (or pass --local)}"
export TF_PLUGIN_CACHE_DIR="$cache/plugins" TF_IN_AUTOMATION=1 CHECKPOINT_DISABLE=1
export TF_VAR_oc_api_url="$OC_TF_API_URL" TF_VAR_oc_app_key="$OC_TF_APP_KEY" TF_VAR_oc_api_key="${OC_TF_API_KEY:-}"
# Only the keys given here: never Datadog credentials from the environment.
unset DD_API_KEY DD_APP_KEY DD_BEARER_TOKEN DATADOG_API_KEY DATADOG_APP_KEY DATADOG_BEARER_TOKEN DD_HOST DATADOG_HOST

auths=(bearer)
if [ -n "${OC_TF_API_KEY:-}" ]; then auths=(keys bearer); else log "OC_TF_API_KEY unset: skipping the api_key + app_key runs"; fi
failed=()
for auth in "${auths[@]}"; do
  (check_wrong_key_rejected "$auth") || failed+=("wrong key rejected ($auth)")
done
for fixture in "${fixtures[@]}"; do
  [ -d "$here/fixtures/$fixture" ] || die "no fixture $fixture"
  for auth in "${auths[@]}"; do
    if ! (run_fixture "$fixture" "$auth"); then
      failed+=("$fixture ($auth)")
      log "FAILED: $fixture ($auth), see $work/$fixture-$auth/run.log:"
      tail -n 30 "$work/$fixture-$auth/run.log" >&2 || true
      (cd "$work/$fixture-$auth" && "$tofu" destroy -auto-approve -input=false -no-color >>run.log 2>&1) || true
      (dir="$work/$fixture-$auth" && teardown_fixture "$fixture") || true
    fi
  done
done
if [ ${#failed[@]} -gt 0 ]; then
  log "failed: ${failed[*]}"
  exit 1
fi
log "all fixtures passed (${fixtures[*]}; ${auths[*]})"
