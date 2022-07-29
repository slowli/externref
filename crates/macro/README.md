# Proc Macro For `externref`

This macro complements the [`externref`] library, wrapping imported or exported functions
with `Resource` args and/or return type. These wrappers are what makes it possible to patch
the generated WASM module with the `externref` processor, so that real `externref`s are used in
argument / return type positions.

## Usage

Add this to your `Crate.toml`:

```toml
[dependencies]
externref-macro = "0.1.0"
```

Note that the `externref` crate re-exports the proc macro if the `macro` crate feature
is enabled (which it is by default). Thus, it is rarely necessary to use this crate
as a direct dependency.

See `externref` docs for more details and examples of usage.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE)
or [MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in `externref` by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.

[`externref`]: https://crates.io/crates/externref
