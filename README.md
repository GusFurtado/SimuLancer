# outliers-sim

Open-source discrete event simulation. Models are plain YAML files: you can diff them, review them, test them and run them headless.

> Early development. Sources, accumulating conveyors, servers with failures, sinks, part types, shared resources, shifts, batches and assembly; validated against queueing theory. The model format is described in [docs/models.md](docs/models.md).

## Usage

```sh
outliers-sim validate examples/conveyor-line.yaml   # check a model, show what was understood
outliers-sim run examples/conveyor-line.yaml        # run the experiment
outliers-sim compare examples/conveyor-line.yaml examples/conveyor-line-longer-c2.yaml
```

A scenario doesn't need to copy the whole model. A scenario file lists only what changes:

```yaml
format: outliers-sim-scenario/0.1
name: two-cells-first-available
base: two-cells.yaml
changes:
  sources.arrivals.output: { rule: first_available }
```

Scenario files work anywhere a model does (`validate`, `run`, `compare`). Errors point at the change that caused them.

`compare` runs both models on the same random numbers (paired replications) and reports each measure's difference with a 95% confidence interval, plus a verdict: higher, lower, or no clear difference. Example: doubling conveyor c2 raises throughput by 1.1 ± 0.1 parts/h, but parts spend 5.0 ± 0.5 min longer in the system.

`run` runs the replications in parallel and prints every measure as mean ± 95% confidence interval. It writes these files to `results/<model name>/`:

- `report.html`: a self-contained report with charts that works offline.
- `summary.json`: every measure with its confidence interval.
- `replications.csv`: raw values per replication.
- `events.jsonl`: an event log, if the model asks for one. See `outliers-sim run --help` for options.

## Server and HTTP API

```sh
outliers-sim serve --data ./data --port 8080
```

Then open `http://localhost:8080` and pick a project (or create one). Each project opens two ways:

- **Modeling.** Draw the model on its diagram (add sources, servers and sinks, connect them with conveyors, drag them into place; conveyor lengths follow the drawing), and click machines and conveyors to change them; errors show next to the field as you type. Run to check, **save a version** with a short message, then **publish** it to the Simulation page. On the *Decisions* tab, choose what can be changed on the Simulation page ("⚑ Make this a decision" under a number, or "Offer as an option…" for a set of edits) and which results it shows. Versions are git commits and tags in the project's repository; nobody needs to know git.
- **Simulation.** Published versions only. Build scenarios from the decisions the model offers (a set of options, or a number within limits), compare them with the current setup, and read the answer in plain language: *"Lead time: 9.10 min → 7.24 min, down 20% (between 18% and 23%) · better"*. Every scenario's report and 2D animation are one click away.

Everything the pages do goes through the documented HTTP API (`docs/api.md`): projects, versions, runs and results as JSON, for scripts and integrations.

Runs execute on the server, one at a time on all cores.

## Deploying

One static Linux binary (about 6 MB, plus git on the system) or a 41 MB Docker image:

```sh
docker run -d -p 127.0.0.1:8080:8080 -v outliers-sim-data:/data ghcr.io/gusfurtado/outliers-sim:latest
```

`docs/deploy.md` covers the systemd service (`deploy/outliers-sim.service`), HTTPS and logins through your reverse proxy (`deploy/nginx.conf`), sizing, backups and upgrades. Tagging a version (`git tag v0.1.0 && git push --tags`) publishes the binary and the image.

## Routing and merging

When a source or server has several outgoing conveyors, or a server has several incoming ones, the model has to say how parts choose. There are no hidden defaults:

```yaml
servers:
  inspect:
    process_time: "uniform(30 s, 50 s)"
    input: { rule: first_come }                                        # or priority, round_robin
    output: { rule: probability, weights: { c_ship: 0.9, c_rework: 0.1 } }  # or round_robin, first_available, shortest_queue, by_type
```

`examples/two-cells.yaml` uses all of them: parallel cells, a merge at inspection, and a rework loop.

## Layout

| Path | What it is |
|---|---|
| `crates/engine` | Simulation core. Standard units only (time in integer µs, length in m), no I/O. |
| `crates/model` | Loads and validates YAML model files and converts units for the engine. |
| `crates/cli` | The `outliers-sim` command (`validate`, `run`). |
| `examples/` | Example models, which are also the templates new projects start from: an assembly line, a bank branch, a warehouse, and smaller ones. `templates.yaml` tags them for the template search, `projects/*.yaml` are their project files (decisions and results), `*.layout.yaml` their drawings, and `blank.yaml` is what a blank project starts as. |
| `viewer/` | Godot 4.7 project that replays event logs in 3D. |
| `DECISIONS.md` | Design decisions and open questions. |
| `proprietary-des-issues.md` | Problems with commercial DES tools that this project is a response to. |

## Viewer

`viewer/` is a Godot 4.7 project that replays a run's event log in 3D. It uses the model's `*.layout.yaml`, or places objects in a row if there is none.

```sh
outliers-sim run examples/conveyor-line.yaml                 # writes events.jsonl + layout.json
godot --path viewer -- results/conveyor-line                 # run from the repository root
```

Started without arguments (or from the Godot editor, desktop or Android), the viewer opens the bundled demo in `viewer/demo/`: replication 1 of the conveyor-line example, regenerated by `viewer/demo/make_demo.sh`.

Controls:

- **Playback:** space plays and pauses; `,` and `.` change the speed (1× to 3600×); drag the timeline to seek.
- **Mouse:** drag to orbit, right-drag to pan, wheel to zoom.
- **Touch:** one finger orbits; two fingers pinch to zoom and drag to pan.

Servers are colored by state, with the same colors as the HTML report.

## Development

```sh
cargo test --workspace
cargo run --release -- run examples/conveyor-line.yaml
viewer/tests/run_all.sh        # viewer checks (needs Godot 4.7; pass its path or set $GODOT)
```

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
