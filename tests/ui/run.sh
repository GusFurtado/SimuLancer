#!/usr/bin/env bash
# Browser test of the web pages: starts `simulancer serve` on a fresh data
# folder (which starts with no projects) and a headless Firefox, then drives
# an engineer's and a manager's session
# (tests/ui/test_ui.py), after checking the animation's replay against the
# engine (tests/replay/test_replay.py). Screenshots go to $SHOTS (default: a temp folder).
#
# Needs Firefox and Python 3. Usage: tests/ui/run.sh
set -euo pipefail
if ! command -v cargo >/dev/null && [ -f "$HOME/.cargo/env" ]; then . "$HOME/.cargo/env"; fi
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# Inside the checkout, not /tmp: a snap-packaged Firefox (as on Ubuntu) has
# a private /tmp and can't use a profile created there.
WORK="$ROOT/target/ui-test"
rm -rf "$WORK"
mkdir -p "$WORK"
SHOTS="${SHOTS:-$WORK/shots}"
PORT="${PORT:-18190}"
MARIONETTE="${MARIONETTE:-2830}"
cleanup() {
  [ -n "${SERVER:-}" ] && kill "$SERVER" 2>/dev/null || true
  [ -n "${BROWSER:-}" ] && kill "$BROWSER" 2>/dev/null || true
  rm -rf "$WORK/data" "$WORK/profile"
}
trap cleanup EXIT

cargo build -q --release --manifest-path "$ROOT/Cargo.toml"
mkdir -p "$WORK/profile"
"$ROOT/target/release/simulancer" serve --data "$WORK/data" --port "$PORT" >"$WORK/server.log" 2>&1 &
SERVER=$!
echo "user_pref(\"marionette.port\", $MARIONETTE);" >"$WORK/profile/user.js"
firefox --headless --marionette --no-remote --profile "$WORK/profile" >"$WORK/firefox.log" 2>&1 &
BROWSER=$!
ready=""
for _ in $(seq 1 300); do
  if (exec 3<>"/dev/tcp/127.0.0.1/$MARIONETTE") 2>/dev/null; then ready=1; break; fi
  sleep 0.2
done
if [ -z "$ready" ]; then
  echo "Firefox's remote control (Marionette) didn't start on port $MARIONETTE within 60 s. Firefox said:"
  cat "$WORK/firefox.log"
  exit 1
fi
# The web animation's replay against the engine, on logs the current CLI writes.
for model in deterministic two-servers routing kits; do
  "$ROOT/target/release/simulancer" run "$ROOT/tests/replay/fixtures/$model.yaml" --output-dir "$WORK/replay/$model" >/dev/null
done
python3 "$ROOT/tests/replay/test_replay.py" "http://127.0.0.1:$PORT/" "$MARIONETTE" "$WORK/replay"
python3 "$ROOT/tests/ui/test_ui.py" "http://127.0.0.1:$PORT/" "$MARIONETTE" "$SHOTS"
echo "screenshots: $SHOTS"
