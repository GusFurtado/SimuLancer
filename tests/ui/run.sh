#!/usr/bin/env bash
# Browser test of the web interface: starts `outliers-sim serve` on a copy of
# the examples and a headless Firefox, then drives a user session
# (tests/ui/test_ui.py). Screenshots go to $SHOTS (default: a temp folder).
#
# Needs Firefox and Python 3. Usage: tests/ui/run.sh
set -euo pipefail
if ! command -v cargo >/dev/null && [ -f "$HOME/.cargo/env" ]; then . "$HOME/.cargo/env"; fi
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
WORK="$(mktemp -d)"
SHOTS="${SHOTS:-$WORK/shots}"
PORT="${PORT:-18190}"
MARIONETTE="${MARIONETTE:-2830}"
cleanup() {
  [ -n "${SERVER:-}" ] && kill "$SERVER" 2>/dev/null || true
  [ -n "${BROWSER:-}" ] && kill "$BROWSER" 2>/dev/null || true
  rm -rf "$WORK/models" "$WORK/runs" "$WORK/profile"
}
trap cleanup EXIT

cargo build -q --release --manifest-path "$ROOT/Cargo.toml"
mkdir -p "$WORK/models" "$WORK/profile"
cp "$ROOT"/examples/*.yaml "$WORK/models/"
"$ROOT/target/release/outliers-sim" serve --models "$WORK/models" --runs "$WORK/runs" --port "$PORT" >"$WORK/server.log" 2>&1 &
SERVER=$!
echo "user_pref(\"marionette.port\", $MARIONETTE);" >"$WORK/profile/user.js"
firefox --headless --marionette --no-remote --profile "$WORK/profile" >"$WORK/firefox.log" 2>&1 &
BROWSER=$!
for _ in $(seq 1 50); do
  if (exec 3<>"/dev/tcp/127.0.0.1/$MARIONETTE") 2>/dev/null; then break; fi
  sleep 0.2
done
python3 "$ROOT/tests/ui/test_ui.py" "http://127.0.0.1:$PORT/" "$MARIONETTE" "$SHOTS"
echo "screenshots: $SHOTS"
