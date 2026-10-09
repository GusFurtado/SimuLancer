#!/usr/bin/env bash
# Headless checks for the viewer:
#   1. import the project (generates .uid files),
#   2. replay logs made by the current CLI and compare with the engine's state,
#   3. run the main scene for a few hundred frames with and without a layout.
# Fails on a non-zero exit code or any ERROR / SCRIPT ERROR in Godot's output.
#
# Usage: viewer/tests/run_all.sh [path/to/godot]   (or set $GODOT)
set -euo pipefail

VIEWER="$(cd "$(dirname "$0")/.." && pwd)"
ROOT="$(dirname "$VIEWER")"
GODOT="${1:-${GODOT:-$HOME/.local/share/godot-bin/Godot_v4.7.2-stable_linux.x86_64}}"
OUT="$(mktemp -d)"
trap 'rm -rf "$OUT"' EXIT

step() {
  local name="$1"; shift
  local log="$OUT/$name.log"
  if ! timeout 300 "$@" >"$log" 2>&1 || grep -qE "^(ERROR|SCRIPT ERROR)|^\s+at: " "$log"; then
    echo "FAIL: $name"
    cat "$log"
    exit 1
  fi
  echo "ok: $name"
}

FIXTURES="$VIEWER/tests/fixtures"
for model in deterministic two-servers routing; do
  step "simulate-$model" cargo run -q --release --manifest-path "$ROOT/Cargo.toml" -- \
    run "$FIXTURES/$model.yaml" --output-dir "$OUT/$model"
done

step import "$GODOT" --headless --path "$VIEWER" --import
step replay "$GODOT" --headless --path "$VIEWER" -s res://tests/test_replay.gd -- \
  "$OUT/deterministic" "$OUT/two-servers" "$OUT/routing"
step scene-with-layout "$GODOT" --headless --path "$VIEWER" --quit-after 300 -- "$OUT/deterministic"
# A relative path, as in the README: resolved against where Godot was started.
(cd "$OUT" && step scene-auto-layout-relative "$GODOT" --headless --path "$VIEWER" --quit-after 300 -- two-servers)
step scene-auto-layout-routing "$GODOT" --headless --path "$VIEWER" --quit-after 300 -- "$OUT/routing"
# No arguments: the bundled demo (gzipped log).
step scene-demo "$GODOT" --headless --path "$VIEWER" --quit-after 300
# The committed demo must match what the current CLI produces.
mkdir -p "$OUT/demo-check"
cp -r "$VIEWER/demo/." "$OUT/demo-check/"
"$VIEWER/demo/make_demo.sh" >/dev/null
if cmp -s "$OUT/demo-check/events.jsonl.gz" "$VIEWER/demo/events.jsonl.gz" \
  && cmp -s "$OUT/demo-check/layout.json" "$VIEWER/demo/layout.json"; then
  echo "ok: demo-up-to-date"
else
  echo "FAIL: viewer/demo was stale and has been regenerated; commit the new files"
  exit 1
fi
# Opening something that isn't there must fail loudly.
if "$GODOT" --headless --path "$VIEWER" --quit-after 30 -- "$OUT/missing" 2>&1 | grep -q "^ERROR"; then
  echo "ok: missing-folder-is-an-error"
else
  echo "FAIL: opening a missing folder didn't report an error"
  exit 1
fi
echo "all viewer checks passed"
