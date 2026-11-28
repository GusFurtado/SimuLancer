# Architecture

SimuLancer is one Rust workspace with three crates, a web app compiled into the binary, and plain files on disk. This page is the map: what lives where, how a model becomes results, and the rules that keep results reproducible.

```text
model.yaml ──> simulancer-model ──> Spec ──> simulancer-engine ──> results per replication
  (YAML,        (parse, check,      (standard   (events, random        │
   units)        convert units)      units)      streams, stats)       ▼
                                                              simulancer (CLI and server)
                                                              measures, checks, files:
                                                              summary.json, replications.csv,
                                                              report.html, events.jsonl
```

## The crates

### `simulancer-engine` (`crates/engine`)

The simulation core. It knows nothing about files, YAML or units: time is integer microseconds (`SimTime`), lengths are meters, and every input is a `Spec`.

| Module | What it does |
|---|---|
| `spec` | The validated model the engine runs: sources, servers, conveyors (and queues and transports, which are kinds of connection), sinks, resources, schedules, vehicles, lines, the experiment. Objects refer to each other by index. |
| `sim` | One replication: the event loop, and the rules of every object (blocking after service, accumulating conveyors, failures, shifts, resources, routing). Its header lists the rules. |
| `events` | The future event list. Events at the same time run in the order they were scheduled. |
| `rng` | Random numbers (xoshiro256++) and every distribution, implemented here so a seed gives the same results whatever the dependency versions. |
| `stats` | Means with Student-t intervals, with the t quantile computed here. |
| `experiment` | Replications in parallel, with results independent of the thread count. |
| `results`, `trace` | What a replication measures, and the events an observer (the event log, the animation) can follow. |

### `simulancer-model` (`crates/model`)

Turns text into a `Spec`, or into a list of problems with their line and column.

| Module | What it does |
|---|---|
| `raw` | The YAML as written, one-to-one; unknown keys are errors. |
| `build` | Checks everything (names, connections, units, reachability, part types, stock, carried resources) and builds the `Spec`, collecting every problem instead of stopping at the first. |
| `units` | Quantities with units, converted once, at load time. The only place that knows about units. |
| `expr`, `formula` | Distribution expressions (`triangular(1 min, 2 min, 4 min)`) and attribute formulas (`rework >= 2 and type == frame`). |
| `scenario` | Scenario files: a base model plus changes at dotted paths, with errors pointing at the change. |
| `layout` | `*.layout.yaml`: where things are drawn. The engine never reads it. |
| `yaml_out` | Writes a model tree back as readable YAML, for the Modeling page's saves. |

### `simulancer` (`crates/cli`)

The binary: the command line, the server and everything around a run.

| Module | What it does |
|---|---|
| `main` | The commands: `validate`, `run`, `compare`, `serve`. |
| `runner` | Runs a model's experiment into a results folder; shared by `run` and the server. |
| `measures` | Turns replication results into named measures with units (throughput, busy, waiting time...). |
| `output`, `report.html`, `report` | The result files, the self-contained HTML report (a template with the results embedded as JSON) and the terminal tables. |
| `warmup`, `precision` | The warm-up check (Welch, MSER) and the replications check. |
| `compare`, `plain` | Paired comparisons on common random numbers, and the same in plain language for the Simulation page. |
| `projects`, `scenarios`, `decisions` | Projects as git repositories (save = commit, publish = tag), managers' scenarios, and what a project exposes to them. |
| `server` | The HTTP API (axum) and the web pages, with the queue of runs. |
| `web/` | The pages: plain HTML, CSS and JavaScript, compiled into the binary. No build step, no npm, nothing loaded from the internet. |

## On disk

```text
<data>/
  projects/<id>/        one git repository per project
    project.yaml          name, which file is the model, decisions and results
    model.yaml            the model
    model.layout.yaml     its drawing (optional)
    README.md             its documentation (optional)
  projects/.archive/    archived projects
  scenarios/<id>.json   managers' scenarios for each project
  runs/<run>/           each run's result files and run.json
  runs/comparisons/     each comparison
```

There's no database. Runs come back after a restart from their `run.json`; projects are read from git, at a tag for published versions, so later edits never change what managers see.

## Reproducibility

- **Integer time.** Simultaneous events are exactly simultaneous, and the tie rule is fixed.
- **Named random streams.** Each stream comes from the seed, the replication number and the name of the object that draws from it. Changing one object doesn't change another's random numbers, which is what makes comparisons fair.
- **No hidden state.** The same model, seed and version give byte-identical result files, on any number of threads. Every example's results are pinned in `tests/pinned/` and checked on every build.

## Where to start

- A new **modeling feature** usually touches `raw` (the key), `build` (checks and conversion), `spec` and `sim` (the behavior), `measures` (what it reports), a case in `crates/engine/tests/`, and [Model files](models.md).
- A new **result** touches `measures`, maybe the report template, and [Results](results.md).
- A new **page feature** touches `server.rs` (a route) and `web/`, with a browser test in `tests/ui/test_ui.py`.

Design decisions, including the ones that were rejected, are in [`DECISIONS.md`](https://github.com/GusFurtado/SimuLancer/blob/main/DECISIONS.md).
