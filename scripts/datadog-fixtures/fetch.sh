#!/bin/sh
# Downloads the Datadog API recordings the contract tests replay (src/routes/datadog_api/replay.rs).
#
# They come from DataDog/datadog-api-client-go (Apache-2.0) at a pinned commit and are not
# redistributed with ObserveCraft: this script fetches that commit's source archive, checks its
# SHA-256 and extracts only what the tests read, as they are upstream:
#   - tests/scenarios/features/{v1,v2}/<feature>.feature for the features below, given.json and the
#     request bodies those features read from files;
#   - tests/scenarios/cassettes/TestScenarios/{v1,v2}/Feature_<Name>/ of those features;
#   - LICENSE and NOTICE.
#
# Destination: $OC_DATADOG_FIXTURES, or ${XDG_CACHE_HOME:-$HOME/.cache}/observecraft/datadog-api-client-go/<commit>.
# The tests read the same location. A complete extraction is marked by a `.complete` file holding the
# commit and the feature list; it is left alone while both match, and extracted again when a feature
# was added; the files are staged next to it and moved into place at the end.
#
# Usage: scripts/datadog-fixtures/fetch.sh   (or `make datadog-fixtures`)
set -eu

COMMIT=1a5c98969c6a269710149bfa7ca7e39320a1e644
SHA256=1aeda444127a7a5005bc16fe0103377b2a778c429eb6583a1e703ee1510824cf
URL="https://codeload.github.com/DataDog/datadog-api-client-go/tar.gz/$COMMIT"

# Features replayed by the tests (`Feature::load` paths).
FEATURES="v1/authentication v1/monitors v1/downtimes v1/dashboards v1/dashboard_lists v1/service_level_objectives
v1/service_level_objective_corrections v2/downtimes v2/dashboard_lists v2/users v2/roles v2/teams v2/key_management
v2/cloud_cost_management"

DEST="${OC_DATADOG_FIXTURES:-${XDG_CACHE_HOME:-$HOME/.cache}/observecraft/datadog-api-client-go/$COMMIT}"

MARK="$COMMIT $(echo $FEATURES)"
if [ -f "$DEST/.complete" ]; then
  if [ "$(cat "$DEST/.complete")" = "$MARK" ]; then
    echo "Datadog API fixtures already in $DEST"
    exit 0
  fi
  echo "The feature list changed: extracting the fixtures again"
  rm -rf "$DEST"
fi
if [ -e "$DEST" ] && [ -n "$(ls -A "$DEST" 2>/dev/null)" ]; then
  echo "$DEST exists but is not a complete fixture extraction (no .complete): remove it and run again" >&2
  exit 1
fi

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

mkdir -p "$(dirname "$DEST")"
TMP="$(mktemp -d "$(dirname "$DEST")/.fetch.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT INT TERM

echo "Downloading datadog-api-client-go@$COMMIT"
curl -fsSL --retry 3 -o "$TMP/source.tar.gz" "$URL"
GOT="$(sha256 "$TMP/source.tar.gz")"
if [ "$GOT" != "$SHA256" ]; then
  echo "SHA-256 mismatch for $URL: got $GOT, expected $SHA256" >&2
  exit 1
fi

# The archive's top directory, and the scenario tree under it.
ROOT="datadog-api-client-go-$COMMIT"
SC="$ROOT/tests/scenarios"
OUT="$TMP/out"
mkdir -p "$OUT"
extract() { tar -xzf "$TMP/source.tar.gz" -C "$OUT" "$@"; }

# Pass 1: licenses, features and their given steps.
set -- "$ROOT/LICENSE" "$ROOT/NOTICE" "$SC/features/v1/given.json" "$SC/features/v2/given.json"
for f in $FEATURES; do set -- "$@" "$SC/features/$f.feature"; done
extract "$@"

# Pass 2: each feature's cassettes (named after its `Feature:` line) and the bodies it reads from files.
set --
for f in $FEATURES; do
  version="${f%%/*}"
  file="$OUT/$SC/features/$f.feature"
  name="$(sed -n 's/^Feature:[[:space:]]*//p' "$file" | head -n 1 | sed 's/[[:space:]]*$//' | tr ' ' '_')"
  [ -n "$name" ] || { echo "no Feature: line in $f.feature" >&2; exit 1; }
  set -- "$@" "$SC/cassettes/TestScenarios/$version/Feature_$name"
  for body in $(sed -n 's/.*from file "\([^"]*\)".*/\1/p' "$file" | sort -u); do
    set -- "$@" "$SC/features/$version/$body"
  done
done
extract "$@"

mkdir -p "$TMP/stage/tests"
mv "$OUT/$ROOT/LICENSE" "$OUT/$ROOT/NOTICE" "$TMP/stage/"
mv "$OUT/$SC" "$TMP/stage/tests/scenarios"
echo "$MARK" >"$TMP/stage/.complete"
rmdir "$DEST" 2>/dev/null || true
mv "$TMP/stage" "$DEST"
echo "Datadog API fixtures extracted to $DEST ($(find "$DEST" -name '*.yaml' | wc -l | tr -d ' ') cassettes)"
