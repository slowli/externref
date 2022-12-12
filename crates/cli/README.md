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

## Usage

The executable provides the same functionality as the WASM [`processor`]
from the `externref` crate. See its docs and the output of `externref --help`
for a detailed description of available options.

> **Important.** The processor should run before WASM optimization tools such as
> `wasm-opt` from binaryen.

### Examples

The terminal capture below demonstrates transforming a test WASM module.
The capture includes the tracing output, which was switched on
by setting the `RUST_LOG` env variable. Tracing info includes each transformed function
and some other information that could be useful for debugging.

![Output with tracing](tests/snapshots/with-tracing.svg)
<!-- TODO: include absolute link before publishing -->

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE)
or [MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in `externref` by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.

[`externref`]: https://crates.io/crates/externref
[`processor`]: https://slowli.github.io/externref/externref/processor/