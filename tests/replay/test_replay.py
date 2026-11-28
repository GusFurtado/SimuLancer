"""The web animation's replay (Replay in common.js) must reproduce the engine's
state: exact conveyor positions, accumulation and blocking, routing, kits,
batches and shifts. It runs in the browser on event logs the current CLI wrote
for the models in tests/replay/fixtures/.

Usage: test_replay.py <server url> <marionette port> <results dir>
Run by tests/ui/run.sh, with <results dir>/<fixture>/events.jsonl for each fixture.
"""
import json, os, sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "ui"))
from marionette import Browser

url, port, results = sys.argv[1].rstrip("/"), int(sys.argv[2]), sys.argv[3]
b = Browser(port)
b.go(url + "/")
b.wait("return window.eval('typeof Replay') === 'function'")

# Shared by every check: a replay from the log's text, and checks that collect failures.
PRELUDE = """
  // Replay is a top-level class in common.js: a global binding, not a property of window.
  const Replay = window.eval("Replay");
  const lines = arguments[0].split("\\n").filter(Boolean).map(l => JSON.parse(l));
  const r = new Replay(lines.shift(), lines);
  const failures = [];
  const check = (ok, what) => { if (!ok) failures.push(what); };
  const near = (a, b) => a.length === b.length && a.every((x, i) => Math.abs(x - b[i]) < 1e-6);
  const positions = (name, expected, what) => check(near(r.belt(name), expected), `${what}: positions ${JSON.stringify(r.belt(name))}, expected ${JSON.stringify(expected)}`);
  const parts = s => [...r.servers[s].busy, ...r.servers[s].blocked, ...r.servers[s].waiting];
  const inSystem = () => r.created + r.assembled + r.split - r.consumed - r.lost - r.exited;
  // Every part in the system is somewhere visible, except one each blocked source may hold.
  const allVisible = what => {
    let visible = 0;
    for (const c of Object.keys(r.conveyors)) visible += r.belt(c).length + r.conveyors[c].pickup.length;
    for (const s of Object.keys(r.servers)) visible += parts(s).length;
    const hidden = inSystem() - visible;
    check(hidden >= 0 && hidden <= r.header.sources.length, `${what}: ${inSystem()} in system, ${visible} visible`);
  };
"""

CHECKS = {
    "deterministic": """
  check(r.header.model === "deterministic", "header names the model");
  check(r.end === 3600, "end time is the run length");
  r.seek(1);  // part 0 is halfway along c_in
  positions("c_in", [1], "t=1 c_in");
  check(r.state("machine") === "idle", "t=1 machine idle");
  check(r.created === 1, "t=1 one part created");
  r.seek(10);  // processing from 2 s to 32 s
  positions("c_in", [], "t=10 c_in");
  check(r.state("machine") === "busy", "t=10 machine busy");
  check(JSON.stringify(parts("machine")) === "[0]", "t=10 part 0 in the machine");
  r.seek(33);  // on c_out since 32 s
  positions("c_out", [1], "t=33 c_out");
  check(r.state("machine") === "idle", "t=33 machine idle");
  r.seek(35);  // exited at 34 s
  check(r.exited === 1 && inSystem() === 0, "t=35 part 0 exited");
  r.seek(61.5);  // part 1 created at 60 s
  positions("c_in", [1.5], "t=61.5 c_in");
  r.seek(1);  // seeking backward replays from the start
  positions("c_in", [1], "back to t=1 c_in");
  check(r.exited === 0 && r.created === 1, "back to t=1 counters");
  r.seek(r.end);  // 60 parts, the last exits at 3574 s
  check(r.created === 60 && r.exited === 60, `end: 60 created, 60 exited (got ${r.created}, ${r.exited})`);
""",
    "two-servers": """
  // A moment when the slow server is mid-way through a part: just before it
  // takes the next one, everything upstream is accumulated.
  const take = r.events.find(e => e.type === "started_process" && e.server === "slow" && e.t > 3000)?.t ?? -1;
  check(take > 0, "slow server takes a part after 3000 s");
  r.seek(take - 1);
  // Capacity 4 (2 m / 0.5 m): head at the end, then one spacing apart.
  positions("c2", [2, 1.5, 1, 0.5], "c2 accumulated");
  positions("c1", [2, 1.5, 1, 0.5], "c1 accumulated");
  check(r.state("fast") === "blocked", "fast server blocked");
  check(r.state("slow") === "busy", "slow server busy");
  // When the head leaves, the last part is already one spacing in, so the
  // fast server's blocked part gets on at once and everyone moves up.
  r.seek(take + 0.25);
  positions("c2", [1.75, 1.25, 0.75, 0.25], "c2 moving up after the head left");
  check(r.state("fast") === "busy", "fast server unblocked and took the next part");
  r.seek(take + 1);  // closed up again
  positions("c2", [2, 1.5, 1, 0.5], "c2 closed up");
""",
    "routing": """
  for (const t of [600, 1800, 3600, 5400, r.end]) { r.seek(t); allVisible(`t=${t}`); }
  const used = new Set(r.events.filter(e => e.type === "entered_conveyor").map(e => e.conveyor));
  for (const c of ["in_a", "in_b", "rework", "retest", "ship"]) check(used.has(c), `some part used conveyor ${c}`);
""",
    "non-accumulating": """
  // Moving: parts on at 3, 6, 9 and 12 s (part 0 went into the server at 10 s).
  r.seek(12);
  positions("belt", [9, 6, 3, 0], "t=12 moving");
  // Part 1 reached the end at 13 s and waits: the belt stopped with its gaps,
  // and the part put on at 15 s stays at the entrance.
  r.seek(50);
  positions("belt", [10, 7, 4, 1, 0], "t=50 stopped with its gaps");
  // At 110 s the server takes part 1 and the belt moves on together.
  r.seek(110.5);
  positions("belt", [7.5, 4.5, 1.5, 0.5], "t=110.5 moving again");
""",
    "kits": """
  let offSeen = false;
  for (const t of [1800, 3600, 23.5 * 3600, 24.5 * 3600, r.end]) {
    r.seek(t);
    allVisible(`kits t=${t}`);
    offSeen = offSeen || Object.keys(r.servers).some(s => r.state(s) === "off");
  }
  check(offSeen, "the oven was seen off shift");
  check(r.assembled > 0 && r.consumed === 3 * r.assembled, `kits: ${r.assembled} assembled, ${r.consumed} consumed`);
""",
}

failed = 0
for name, checks in CHECKS.items():
    with open(os.path.join(results, name, "events.jsonl")) as f:
        text = f.read()
    failures = b.js(PRELUDE + checks + "\n  return failures;", text)
    for what in failures:
        print(f"FAIL {name}: {what}")
    failed += len(failures)
    if not failures:
        print(f"ok: replay {name}")
if failed:
    raise SystemExit(1)
print("replay: all checks passed")
