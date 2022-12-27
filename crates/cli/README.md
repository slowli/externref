# CLI for `externref` Crate

[![Build Status](https://github.com/slowli/externref/workflows/CI/badge.svg?branch=main)](https://github.com/slowli/externref/actions)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%2FApache--2.0-blue)](https://github.com/slowli/externref#license)
![rust 1.60+ required](https://img.shields.io/badge/rust-1.60+-blue.svg?label=Required%20Rust)

This crate provides command-line interface for [`externref`]. It allows transforming
WASM modules that use `externref` shims to use real `externref` types.

## Installation

Install with

```shell
cargo install --locked externref-cli
# This will install `externref` executable, which can be checked
# as follows:
externref --help
```

By default, tracing is enabled via the `tracing` crate feature. You can disable
the feature manually by adding a `--no-default-features` arg to the installation command.
Tracing is performed with the `externref::*` targets, mostly on the `DEBUG` and `INFO` levels.
Tracing events are output to the stderr using [the standard subscriber][fmt-subscriber];
its filtering can be configured using the `RUST_LOG` env variable
(e.g., `RUST_LOG=externref=debug`).

## Usage

The executable provides the same functionality as the WASM [`processor`]
from the `externref` crate. See its docs and the output of `externref --help`
for a detailed description of available options.

> **Warning**
>
> The processor should run before WASM optimization tools such as
> `wasm-opt` from binaryen.

### Using Docker image

As a lower-cost alternative to the local installation, you may install and use the CLI app
from the [GitHub Container registry](https://github.com/slowli/externref/pkgs/container/externref).
To run the app in a Docker container, use a command like

```shell
cat module.wasm | \
  docker run -i --rm ghcr.io/slowli/externref:main \
  /externref - > processed-module.wasm
```

Here, `/externref -` specifies the executed command in the Docker image
and its argument (reading the input module from the stdin).
To output tracing information, set the `RUST_LOG` env variable in the container,
e.g. using `docker run --env RUST_LOG=debug ..`.

### Examples

The terminal capture below demonstrates transforming a test WASM module.
The capture includes the tracing output, which was switched on
by setting the `RUST_LOG` env variable. Tracing info includes each transformed function
and some other information that could be useful for debugging.

![Output with tracing][output-with-tracing]

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE)
or [MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in `externref` by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.

[`externref`]: https://crates.io/crates/externref
[fmt-subscriber]: https://docs.rs/tracing-subscriber/latest/tracing_subscriber/fmt/index.html
[`processor`]: https://slowli.github.io/externref/externref/processor/
[output-with-tracing]: https://github.com/slowli/externref/raw/HEAD/crates/cli/tests/snapshots/with-tracing.svg?sanitize=true
