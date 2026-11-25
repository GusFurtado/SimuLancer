<img src="docs/simulancer.png" alt="" width="96" align="right">

# SimuLancer

Open-source discrete event simulation. Models are plain YAML files: you can diff them, review them, test them and run them headless.

**Documentation:** [gustavofurtado.com/SimuLancer](https://www.gustavofurtado.com/SimuLancer/), with a tutorial, the model format, the methods and the HTTP API.

> Early development. Sources, accumulating conveyors, queues, servers with failures, sinks, part types, part attributes set with formulas, shared resources, shifts, batches and assembly; validated against queueing theory. The model format is described in [docs/models.md](docs/models.md).

## Usage

```sh
simulancer validate examples/conveyor-line.yaml   # check a model, show what was understood
simulancer run examples/conveyor-line.yaml        # run the experiment
simulancer compare examples/conveyor-line.yaml examples/conveyor-line-longer-c2.yaml
```

A scenario doesn't need to copy the whole model. A scenario file lists only what changes:

```yaml
format: simulancer-scenario/0.1
name: two-cells-first-available
base: two-cells.yaml
changes:
  sources.arrivals.output: { rule: first_available }
```

Scenario files work anywhere a model does (`validate`, `run`, `compare`). Errors point at the change that caused them.

`compare` runs both models on the same random numbers (paired replications) and reports each measure's difference with a 95% confidence interval, plus a verdict: higher, lower, or no clear difference. Example: doubling conveyor c2 raises throughput by 1.1 ± 0.1 parts/h, but parts spend 5.0 ± 0.5 min longer in the system.

`run` runs the replications in parallel and prints every measure as mean ± 95% confidence interval. It writes these files to `results/<model name>/`:

- `report.html`: a self-contained report with charts that works offline.
- `summary.json`: every measure with its confidence interval, and the warm-up check (parts in the system over time).
- `replications.csv`: raw values per replication.
- `events.jsonl`: an event log, if the model asks for one. See `simulancer run --help` for options.

## Server and HTTP API

```sh
simulancer serve --data ./data --port 8080
```

Then open `http://localhost:8080` and pick a project (or create one). Each project opens two ways:

- **Modeling.** Draw the model on its diagram (drag sources, servers and sinks from the Components panel onto it, connect them by dragging from the dot on one object to the next and choosing a conveyor, queue or transport, drag them into place; conveyor lengths follow the drawing), and click machines, conveyors and queues to change them; part types, attributes, resources, vehicles and schedules are under *Definitions*, in the bar on the left; errors show next to the field as you type. Run to check, **save a version** with a short message, then **publish** it to the Simulation page. On the *Decisions* tab, choose what can be changed on the Simulation page ("⚑ Make this a decision" under a number, or "Offer as an option…" for a set of edits) and which results it shows. On the *Documentation* tab, write down in Markdown what the model can't say for itself: assumptions, business rules, where the data comes from. Versions are git commits and tags in the project's repository; nobody needs to know git.
- **Simulation.** Published versions only. Build scenarios from the decisions the model offers (a set of options, or a number within limits), compare them with the current setup, and read the answer in plain language: *"Lead time: 9.10 min → 7.24 min, down 20% (between 18% and 23%) · better"*. Every scenario's report and 2D animation are one click away.

Everything the pages do goes through the documented HTTP API (`docs/api.md`): projects, versions, runs and results as JSON, for scripts and integrations.

Runs execute on the server, one at a time on all cores.

## Deploying

One static Linux binary (about 6 MB, plus git on the system) or a 41 MB Docker image:

```sh
docker run -d -p 127.0.0.1:8080:8080 -v simulancer-data:/data ghcr.io/gusfurtado/simulancer:latest
```

`docs/deploy.md` covers the systemd service (`deploy/simulancer.service`), HTTPS and logins through your reverse proxy (`deploy/nginx.conf`), sizing, backups and upgrades. Tagging a version (`git tag v0.1.0 && git push --tags`) publishes the binary and the image.

## Routing and merging

When a source or server has several outgoing conveyors or queues, or a server has several incoming ones, the model has to say how parts choose. There are no hidden defaults:

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
| `crates/cli` | The `simulancer` command (`validate`, `run`, `compare`, `serve`); the web pages are in `src/web/`, compiled into the binary. |
| `examples/` | Example models, which are also the templates new projects start from: an assembly line, a bank branch, a warehouse, and smaller ones. `templates.yaml` tags them for the template search, `projects/*.yaml` are their project files (decisions and results), `projects/*.md` their documentation, `*.layout.yaml` their drawings, and `blank.yaml` is what a blank project starts as. |
| `tests/` | Browser tests: `replay/` checks the animation's replay against the engine, `ui/` drives the web pages. |
| `docs/`, `mkdocs.yml` | The documentation site (MkDocs Material): `mkdocs serve` to preview it. Published from `main` by `.github/workflows/docs.yml`. |
| `deploy/` | A systemd unit and an nginx example, described in `docs/deploy.md`. |
| `DECISIONS.md` | Design decisions and open questions. |
| `proprietary-des-issues.md` | Problems with commercial DES tools that this project is a response to. |

## Development

```sh
cargo test --workspace
cargo run --release -- run examples/conveyor-line.yaml
tests/ui/run.sh                # browser tests: the animation's replay, then the web pages (needs Firefox)
```

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
