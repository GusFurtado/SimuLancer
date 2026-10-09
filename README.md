# outliers-sim

Open-source discrete event simulation. Models are plain YAML files: you can diff them, review them, test them and run them headless.

> Early development. Sources, accumulating conveyors, servers with failures and sinks; validated against queueing theory.

## Usage

```sh
outliers-sim validate examples/conveyor-line.yaml   # check a model, show what was understood
outliers-sim run examples/conveyor-line.yaml        # run the experiment
```

`run` runs the replications in parallel and prints every measure as mean ± 95% confidence interval. It writes these files to `results/<model name>/`:

- `report.html`: a self-contained report with charts that works offline.
- `summary.json`: every measure with its confidence interval.
- `replications.csv`: raw values per replication.
- `events.jsonl`: an event log, if the model asks for one. See `outliers-sim run --help` for options.

## Layout

| Path | What it is |
|---|---|
| `crates/engine` | Simulation core. Standard units only (time in integer µs, length in m), no I/O. |
| `crates/model` | Loads and validates YAML model files and converts units for the engine. |
| `crates/cli` | The `outliers-sim` command (`validate`, `run`). |
| `examples/` | Example models. `*.layout.yaml` files are read only by the viewer. |
| `DECISIONS.md` | Design decisions and open questions. |
| `proprietary-des-issues.md` | Problems with commercial DES tools that this project is a response to. |

## Development

```sh
cargo test --workspace
cargo run --release -- run examples/conveyor-line.yaml
```

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
