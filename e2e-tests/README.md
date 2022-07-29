# E2E Tests For `externref`

This crate provides end-to-end tests for the `externref` crate.
Testing works by defining a WASM module that uses `externref::Resource`s,
processing this module with `externref::processor`, and then running
this module using [`wasmtime`].

[`wasmtime`]: https://crates.io/crates/wasmtime
