#!/usr/bin/env bash
# Retakes the documentation's screenshots (docs/images/): starts
# `simulancer serve` on a fresh data folder and a headless Firefox, runs the
# conveyor-line example for its warm-up check, then builds the tutorial's
# model through the web pages (tests/ui/docs_screenshots.py). Run it after a
# change to the pages, and check the images before committing them.
#
# Needs Firefox and Python 3 with Pillow (for WebP). Usage: tests/ui/docs-screenshots.sh
set -euo pipefail
if ! command -v cargo >/dev/null && [ -f "$HOME/.cargo/env" ]; then . "$HOME/.cargo/env"; fi
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# Inside the checkout, not /tmp: a snap-packaged Firefox has a private /tmp.
WORK="$ROOT/target/docs-screenshots"
rm -rf "$WORK"
mkdir -p "$WORK/profile" "$WORK/shots"
PORT="${PORT:-18191}"
MARIONETTE="${MARIONETTE:-2831}"
cleanup() {
  [ -n "${SERVER:-}" ] && kill "$SERVER" 2>/dev/null || true
  [ -n "${BROWSER:-}" ] && kill "$BROWSER" 2>/dev/null || true
  rm -rf "$WORK/data" "$WORK/profile"
}
trap cleanup EXIT

cargo build -q --release --manifest-path "$ROOT/Cargo.toml"
"$ROOT/target/release/simulancer" run "$ROOT/examples/conveyor-line.yaml" --output-dir "$WORK/conveyor-line" >/dev/null 2>&1
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
python3 "$ROOT/tests/ui/docs_screenshots.py" "http://127.0.0.1:$PORT/" "$MARIONETTE" "$WORK/shots" "$WORK/conveyor-line/report.html"
python3 - "$WORK/shots" "$ROOT/docs/images" <<'EOF'
import pathlib, sys
from PIL import Image
shots, images = map(pathlib.Path, sys.argv[1:])
for png in sorted(shots.glob("*.png")):
    to = images / (png.stem + ".webp") if png.stem == "warmup-check" else images / "tutorial" / (png.stem + ".webp")
    Image.open(png).convert("RGB").save(to, quality=88, method=6)
    print(to.relative_to(images.parent.parent))
EOF
