#!/usr/bin/env bash
# Regenerates the demo the viewer opens when started without arguments:
# replication 1 of examples/conveyor-line.yaml. The log is gzipped (about
# 650 KB instead of 6 MB); `gzip -n` keeps the file identical between runs.
set -euo pipefail
DEMO="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$DEMO/../.." && pwd)"
OUT="$(mktemp -d)"
trap 'rm -rf "$OUT"' EXIT

cargo run -q --release --manifest-path "$ROOT/Cargo.toml" -- \
  run "$ROOT/examples/conveyor-line.yaml" --replications 1 --output-dir "$OUT" >/dev/null 2>&1
gzip -9 -n -c "$OUT/events.jsonl" >"$DEMO/events.jsonl.gz"
cp "$OUT/layout.json" "$DEMO/layout.json"
ls -la "$DEMO"
