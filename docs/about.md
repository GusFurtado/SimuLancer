# About

## License

SimuLancer is licensed under either of the
[Apache License 2.0](https://github.com/GusFurtado/SimuLancer/blob/main/LICENSE-APACHE) or the
[MIT license](https://github.com/GusFurtado/SimuLancer/blob/main/LICENSE-MIT), at your option.

## Credits

SimuLancer is written in Rust. The command line uses [clap](https://github.com/clap-rs/clap),
the model files are read with [serde](https://serde.rs) and
[serde-saphyr](https://github.com/bourumir-wyngs/serde-saphyr), the web server runs on
[axum](https://github.com/tokio-rs/axum) and [Tokio](https://tokio.rs), and project
documentation is rendered with [pulldown-cmark](https://github.com/pulldown-cmark/pulldown-cmark).
The web pages use no third-party code and no web fonts. The wordmark is set in
[Montserrat](https://github.com/JulietaUla/Montserrat) (SIL OFL), outlined.

## Contributing

Build and test with Cargo:

```sh
cargo test --workspace
cargo run --release -- run examples/conveyor-line.yaml
tests/ui/run.sh                # browser tests: the animation's replay, then the web pages (needs Firefox)
```

CI also runs `cargo fmt --all --check` and `cargo clippy --workspace --all-targets`, and
validates every example model: run fmt and clippy before opening a pull request.
Design decisions and open questions are recorded in
[`DECISIONS.md`](https://github.com/GusFurtado/SimuLancer/blob/main/DECISIONS.md); read the
relevant section before changing how something works.

This site is built with [MkDocs Material](https://squidfunk.github.io/mkdocs-material/). To
preview it locally:

```sh
pip install -r requirements-docs.txt
mkdocs serve
```

The screenshots are retaken by `tests/ui/docs-screenshots.sh`. The site publishes to GitHub
Pages on every push to `main` that touches the docs.
