#!/bin/sh
# Downloads the log samples the integration pipeline tests replay (src/pipelines/samples.rs).
#
# They are the `assets/logs/<pipeline>_tests.yaml` files of DataDog/integrations-core and DataDog/integrations-extras
# (BSD-3-Clause) at the revisions the pipelines were imported from (PINNED_REVISION and EXTRAS_REVISION in
# scripts/import-integration-dashboards.mjs), and are not redistributed with ObserveCraft: this script fetches each
# revision's source archive, checks its SHA-256 and extracts only those files and the LICENSE, as they are upstream.
#
# Destination: $OC_INTEGRATION_LOG_SAMPLES, or
# ${XDG_CACHE_HOME:-$HOME/.cache}/observecraft/integration-log-samples/<core revision>-<extras revision>, with one
# directory per repository (integrations-core/<check>/assets/logs/…). The tests read the same location. A complete
# extraction is marked by a `.complete` file and left alone.
#
# Usage: scripts/integration-log-samples/fetch.sh   (or `make integration-log-samples`)
set -eu

CORE=1e7420ec635aafdd33db6a1e69586de7d2e24c21
CORE_SHA256=984304711d04f978ca13f455981a55b454ebda5a7992c4dfb0a4b5d53a9ecb49
EXTRAS=20d3ceeaf342b8c9f6211dc9df9ea3d382dbb692
EXTRAS_SHA256=e63df2edcbf4f62534537b05ddbe50bdff3b281456029502e9e422c1eff13f23

DEST="${OC_INTEGRATION_LOG_SAMPLES:-${XDG_CACHE_HOME:-$HOME/.cache}/observecraft/integration-log-samples/$CORE-$EXTRAS}"

if [ -f "$DEST/.complete" ]; then
  echo "Integration log samples already in $DEST"
  exit 0
fi
if [ -e "$DEST" ] && [ -n "$(ls -A "$DEST" 2>/dev/null)" ]; then
  echo "$DEST exists but is not a complete extraction (no .complete): remove it and run again" >&2
  exit 1
fi

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

mkdir -p "$(dirname "$DEST")"
TMP="$(mktemp -d "$(dirname "$DEST")/.fetch.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT INT TERM
mkdir -p "$TMP/stage"

# fetch <repository> <revision> <sha256>
fetch() {
  echo "Downloading DataDog/$1@$2"
  curl -fsSL --retry 3 -o "$TMP/$1.tar.gz" "https://codeload.github.com/DataDog/$1/tar.gz/$2"
  got="$(sha256 "$TMP/$1.tar.gz")"
  if [ "$got" != "$3" ]; then
    echo "SHA-256 mismatch for DataDog/$1@$2: got $got, expected $3" >&2
    exit 1
  fi
  root="$1-$2"
  tar -tzf "$TMP/$1.tar.gz" | grep -E "^$root/(LICENSE|[^/]+/assets/logs/[^/]+_tests\.yaml)\$" >"$TMP/$1.list"
  mkdir -p "$TMP/out"
  # One `tar -T` pass: the file list works with both GNU tar and bsdtar.
  tar -xzf "$TMP/$1.tar.gz" -C "$TMP/out" -T "$TMP/$1.list"
  mv "$TMP/out/$root" "$TMP/stage/$1"
  rm "$TMP/$1.tar.gz"
}
fetch integrations-core "$CORE" "$CORE_SHA256"
fetch integrations-extras "$EXTRAS" "$EXTRAS_SHA256"

echo "$CORE $EXTRAS" >"$TMP/stage/.complete"
rmdir "$DEST" 2>/dev/null || true
mv "$TMP/stage" "$DEST"
echo "Integration log samples extracted to $DEST ($(find "$DEST" -name '*_tests.yaml' | wc -l | tr -d ' ') files)"
