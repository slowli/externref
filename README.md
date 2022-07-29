# Low-Cost Reference Type Shims For WASM Modules

A [reference type] (aka `externref` or `anyref`) is an opaque reference made available to
a WASM module by the host environment. Such references cannot be forged in the WASM code
and can be associated with arbitrary host data, thus making them a good alternative to
ad-hoc handles (e.g., numeric ones). References cannot be stored in WASM linear memory; they are
thus confined to the stack and tables with `externref` elements.

Rust does not support reference types natively; there is no way to produce an import / export
that has `externref` as an argument or a return type. [`wasm-bindgen`] patches WASM if
`externref`s are enabled. This library strives to accomplish the same goal for generic
low-level WASM ABIs (`wasm-bindgen` is specialized for browser hosts).

## Usage

Add this to your `Crate.toml`:

```toml
[dependencies]
externref = "0.1.0"
```

1. Use `Resource`s as arguments / return results for imported and/or exported functions
  in a WASM module in place of `externref`s . Reference args (including mutable references)
  and the `Option<_>` wrapper are supported as well.
2. Add the `#[externref]` proc macro on the imported / exported functions.
3. Transform the generated WASM module with the module processor
  from the corresponding module of the crate.

### Examples

Using the `#[externref]` macro and `Resource`s in WASM-targeting code:

```rust
use externref::{externref, Resource};

// Two marker types for different resources.
pub struct Arena(());
pub struct Bytes(());

#[cfg(target_arch = "wasm32")]
#[externref]
#[link(wasm_import_module = "arena")]
extern "C" {
    // This import will have signature `(externref, i32) -> externref`
    // on host.
    fn alloc(arena: &Resource<Arena>, size: usize) 
        -> Option<Resource<Bytes>>;
}

// Fallback for non-WASM targets.
#[cfg(not(target_arch = "wasm32"))]
unsafe fn alloc(_: &Resource<Arena>, _: usize) 
    -> Option<Resource<Bytes>> { None }

// This export will have signature `(externref) -> ()` on host.
#[externref]
#[export_name = "test_export"]
pub extern "C" fn test_export(arena: &Resource<Arena>) {
    let bytes = unsafe { alloc(arena, 42) }.expect("cannot allocate");
    // Do something with `bytes`...
}
```

See crate docs for more examples of usage and implementation details.

## Project status 🚧

Experimental; it may be the case that the processor produces invalid WASM
in some corner cases (please report this as an issue if it does).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE)
or [MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in `externref` by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.

[reference type]: https://webassembly.github.io/spec/core/syntax/types.html#reference-types
[`wasm-bindgen`]: https://crates.io/crates/wasm-bindgen
