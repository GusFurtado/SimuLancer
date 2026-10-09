# outliers-sim

Open-source discrete event simulation. Models are plain YAML files: you can diff them, review them, test them and run them headless.

> Early development. `validate` works. The engine simulates (sources, accumulating conveyors, servers with failures, sinks), but `run` isn't wired up yet.

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
cargo run -- validate examples/conveyor-line.yaml
```

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
