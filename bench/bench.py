#!/usr/bin/env python3
"""Benchmark: how SimuLancer scales with model size, parts in the system,
run length and the event log.

Generates models (K parallel lines of L stations each, every station with
failures) into target/bench/, runs each with the release binary (one
replication, one thread), and prints a Markdown table: load time, time per
replication, events per second, peak memory and event log size.

Usage: cargo build --release && python3 bench/bench.py [--quick]
Needs only Python 3, on Linux or macOS (for the peak memory).
"""
import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "target" / "release" / ("simulancer.exe" if os.name == "nt" else "simulancer")
WORK = ROOT / "target" / "bench"


def model(name, lines, stations, run_length="7 d", queue_capacity=10, load=0.85, event_log=False):
    """K parallel lines of L stations, fed round robin by one source, each
    station with failures; one sink. `load` is each station's utilization
    before failures."""
    per_line = 1.0  # minutes between parts on each line
    process = per_line * load
    out = [
        "format: simulancer/0.1",
        f"name: {name}",
        "entities:",
        "  part: {}",
        "sources:",
        "  arrivals:",
        "    creates: part",
        f'    interarrival: "exponential({per_line / lines:.6f} min)"',
    ]
    if lines > 1:
        out.append("    output: { rule: round_robin }")
    out.append("servers:")
    for k in range(lines):
        for s in range(stations):
            out.append(
                f'  s{k}_{s}: {{ process_time: "triangular({process * 0.7:.4f} min, {process:.4f} min, {process * 1.3:.4f} min)", '
                f'failures: [{{ name: jam, uptime: "exponential(2 h)", downtime: "uniform(2 min, 8 min)" }}] }}'
            )
    cap = f", capacity: {queue_capacity}" if queue_capacity else ""
    out.append("queues:")
    for k in range(lines):
        out.append(f"  q{k}_0: {{ from: arrivals, to: s{k}_0{cap} }}")
        for s in range(1, stations):
            out.append(f"  q{k}_{s}: {{ from: s{k}_{s - 1}, to: s{k}_{s}{cap} }}")
        out.append(f"  q{k}_out: {{ from: s{k}_{stations - 1}, to: done }}")
    out += [
        "sinks:",
        "  done: {}",
        "experiment:",
        "  replications: 1",
        '  warmup: "1 d"',
        f'  run_length: "{run_length}"',
        "  seed: 1",
        "outputs:",
        f"  event_log: {'jsonl' if event_log else 'none'}",
        "  summary: [throughput, time_in_system, server_states]",
    ]
    return "\n".join(out) + "\n"


def timed(args):
    """Runs a command; returns its wall time (s) and peak memory (MB)."""
    start = time.perf_counter()
    child = subprocess.Popen(args, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    _, status, usage = os.wait4(child.pid, 0)
    elapsed = time.perf_counter() - start
    if status != 0:
        sys.exit(f"{args} failed: {child.stderr.read().decode()}")
    rss = usage.ru_maxrss
    return elapsed, (rss / 1024 / 1024 if sys.platform == "darwin" else rss / 1024)


def measure(name, text, repeat=3):
    path = WORK / f"{name}.yaml"
    path.write_text(text)
    out = WORK / name
    objects = text.count("process_time") * 2 + 3
    # Loading alone (validate), then full runs; the best of `repeat`.
    load, _ = timed([BIN, "validate", path])
    best, peak = None, 0.0
    for _ in range(repeat):
        elapsed, rss = timed([BIN, "run", path, "-t", "1", "-o", out])
        best = elapsed if best is None else min(best, elapsed)
        peak = max(peak, rss)
    summary = json.loads((out / "summary.json").read_text())
    parts = next(m["mean"] for m in summary["measures"] if m["object"] == "done" and m["measure"] == "throughput")
    log = out / "events.jsonl"
    events = sum(1 for _ in log.open()) - 1 if log.exists() else None
    size = log.stat().st_size / 1e6 if log.exists() else None
    return {
        "name": name,
        "objects": objects,
        "load_s": load,
        "run_s": best,
        "parts_per_h": parts,
        "events": events,
        "log_mb": size,
        "peak_mb": peak,
    }


def main():
    quick = "--quick" in sys.argv
    if not BIN.exists():
        sys.exit(f"build it first: cargo build --release ({BIN} is missing)")
    WORK.mkdir(parents=True, exist_ok=True)
    cases = [
        ("objects-100", dict(lines=10, stations=5)),
        ("objects-1000", dict(lines=25, stations=20)),
        ("objects-10000", dict(lines=100, stations=50)),
        ("wip-high", dict(lines=25, stations=20, queue_capacity=None, load=0.97)),
        ("length-1d", dict(lines=25, stations=20, run_length="1 d")),
        ("length-28d", dict(lines=25, stations=20, run_length="28 d")),
        ("event-log", dict(lines=25, stations=20, event_log=True)),
    ]
    if quick:
        cases = cases[:2]
    rows = []
    for name, kw in cases:
        print(f"running {name}...", file=sys.stderr)
        rows.append(measure(name, model(name, **kw), repeat=1 if quick else 3))
    # Events per second: the event-log case counts the events of the same
    # model as objects-1000, whose run has no log.
    events_1000 = next((r["events"] for r in rows if r["name"] == "event-log"), None)
    print("| case | objects | load (s) | one replication (s) | parts/h | events/s | peak memory (MB) | event log (MB) |")
    print("|---|---|---|---|---|---|---|---|")
    for r in rows:
        log = "" if r["log_mb"] is None else f"{r['log_mb']:.0f}"
        eps = ""
        if r["name"] == "objects-1000" and events_1000:
            eps = f"{events_1000 / r['run_s']:,.0f}"
        elif r["events"]:
            eps = f"{r['events'] / r['run_s']:,.0f} (with the log)"
        print(
            f"| {r['name']} | {r['objects']:,} | {r['load_s']:.2f} | {r['run_s']:.2f} | {r['parts_per_h']:,.0f} | {eps} | "
            f"{r['peak_mb']:.0f} | {log} |"
        )


if __name__ == "__main__":
    main()
