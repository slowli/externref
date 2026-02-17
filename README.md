# Low-Cost Reference Type Shims For WASM Modules

[![CI](https://github.com/slowli/externref/actions/workflows/ci.yml/badge.svg)](https://github.com/slowli/externref/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%2FApache--2.0-blue)](https://github.com/slowli/externref#license)
[![The Book](https://img.shields.io/badge/The%20Book-yellow?logo=mdbook)](https://slowli.github.io/externref/)

A [reference type] (aka `externref` or `anyref`) is an opaque reference made available to
a WASM module by the host environment. Such references cannot be forged in the WASM code
and can be associated with arbitrary host data, thus making them a good alternative to
ad-hoc handles (e.g., numeric ones). References cannot be stored in WASM linear memory; they are
confined to the stack and tables with `externref` elements.

Rust does not support reference types natively; there is no way to produce an import / export
that has `externref` as an argument or a return type. [`wasm-bindgen`] patches WASM if
`externref`s are enabled. This library strives to accomplish the same goal for generic
low-level WASM ABIs (`wasm-bindgen` is specialized for browser hosts).

## Project overview

The project consists of the following crates:

- [`externref`](crates/lib): The library providing more typesafe `externref`s for Rust
- [`externref-macro`](crates/macro): Procedural macro for the library
- [`externref-cli`](crates/cli): CLI app for WASM transforms based on the library

## Project status 🚧

Experimental; it may be the case that the processor produces invalid WASM
in some corner cases (please report this as an issue if it does).

## Contributing

All contributions are welcome! See [the contributing guide](CONTRIBUTING.md) to help
you get involved.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE)
or [MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in `externref` by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.

[reference type]: https://webassembly.github.io/spec/core/syntax/types.html#reference-types
[`wasm-bindgen`]: https://crates.io/crates/wasm-bindgen
