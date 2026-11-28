# Contributing to SimuLancer

Thanks for helping. This file says how to build and test SimuLancer, how a pull request should look, and where to find out why things are the way they are. For how the code fits together, read [Architecture](docs/architecture.md) first.

## Build and test

You need stable Rust ([rustup](https://rustup.rs)) and git. The browser tests also need Firefox and Python 3; the docs need Python and `requirements-docs.txt`.

```sh
cargo build                       # debug build; the engine is optimized even here
cargo test --workspace            # unit, engine validation, CLI, server and pinned-results tests
cargo fmt --all                   # CI fails on unformatted code
cargo clippy --workspace --all-targets
tests/ui/run.sh                   # browser tests: the animation's replay, then the web pages
```

CI runs all of these, plus `simulancer validate` on every example. Run fmt and clippy before you push.

### Tests you should know about

- **Engine validation** (`crates/engine/tests/queueing.rs`): models with exact answers from queueing theory. A change to the engine must keep them passing; a new modeling feature should add a case when one exists. See [Validating the engine](docs/methods/validation.md).
- **Pinned results** (`crates/cli/tests/pinned.rs`): every example, run with its seed, must give exactly the `summary.json` in `tests/pinned/`. If your change is *meant* to move results (a bug fix in a distribution, say), regenerate them and say so in the pull request:

  ```sh
  UPDATE_PINNED=1 cargo test -p simulancer --test pinned
  ```

  If it isn't meant to, the failure is a bug.
- **Server** tests live in `crates/cli/src/server.rs` and drive the HTTP API in-process.
- **Browser** tests (`tests/ui/`) drive a headless Firefox through the Modeling and Simulation pages; screenshots land in `target/ui-test/shots`.

## Pull requests

- **One change per pull request**, with a title that says what it does ("Add empirical distributions"), not how.
- **Say what you checked**: which tests, and screenshots for anything visible in the web pages or the report.
- **Docs in the same pull request.** A new key in the model format goes in [Model files](docs/models.md); a new result in [Results](docs/results.md); a new route in [HTTP API](docs/api.md). If it changes the status of a row in [Why open source](docs/why.md), update the row.
- **Design decisions go in [`DECISIONS.md`](DECISIONS.md)**: what was chosen, what was rejected and why. Read the relevant section before changing how something works; many "why not just..." questions are answered there.
- **No new dependencies without a reason.** The engine has none on purpose (its random numbers and statistics are its own, so results don't change with a dependency's version). The web pages load nothing from the internet: no npm, no CDN, no web fonts.
- **Commit messages:** a plain subject line in the imperative, then what changed and why.

## The model format

The format is versioned (`simulancer/0.1`). Until 1.0 a release may change it, and older files are then refused with a message naming the release that reads them; every format change is announced in the release notes. A change that adds an optional key keeps the version. The [format specification](docs/format.md) is the reference for anyone writing another implementation.

## Issues

- **Bugs:** what you ran (the model file, or the smallest one that shows it), what you expected and what happened. `simulancer --version` helps.
- **Features:** the real system you're trying to model and what's missing, more than the solution you have in mind.
- Issues labeled [good first issue](https://github.com/GusFurtado/SimuLancer/labels/good%20first%20issue) are small and self-contained.

## License

SimuLancer is dual-licensed under MIT or Apache 2.0, at your option. Unless you say otherwise, any contribution you submit is licensed the same way, without additional terms or conditions.
