# Building from Sources

To build the CLI app from sources, run:

```bash
cargo install --locked externref-cli
# This will install `externref` executable, which can be checked
# as follows:
externref --help
```

This requires a Rust toolchain locally installed.

## Minimum supported Rust version

The crate supports the latest stable Rust version. It may support previous stable Rust versions,
but this is not guaranteed.

## Crate feature: `tracing`

By default, tracing is enabled via the `tracing` crate feature. You can disable
the feature manually by adding a `--no-default-features` arg to the installation command.
Tracing is performed with the `externref::*` targets, mostly on the `DEBUG` and `INFO` levels.
Tracing events are output to the stderr using [the standard subscriber][fmt-subscriber];
its filtering can be configured using the `RUST_LOG` env variable
(e.g., `RUST_LOG=externref=debug`).

[fmt-subscriber]: https://docs.rs/tracing-subscriber/latest/tracing_subscriber/fmt/index.html
