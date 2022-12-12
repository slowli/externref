# Low-Cost Reference Type Shims For WASM Modules

[![Build Status](https://github.com/slowli/externref/workflows/CI/badge.svg?branch=main)](https://github.com/slowli/externref/actions)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%2FApache--2.0-blue)](https://github.com/slowli/externref#license)
![rust 1.60+ required](https://img.shields.io/badge/rust-1.60+-blue.svg?label=Required%20Rust)

**Documentation:** [![Docs.rs](https://docs.rs/externref/badge.svg)](https://docs.rs/externref/)
[![crate docs (main)](https://img.shields.io/badge/main-yellow.svg?label=docs)](https://slowli.github.io/externref/externref/)

A [reference type] (aka `externref` or `anyref`) is an opaque reference made available to
a WASM module by the host environment. Such references cannot be forged in the WASM code
and can be associated with arbitrary host data, thus making them a good alternative to
ad-hoc handles (e.g., numeric ones). References cannot be stored in WASM linear memory; they are
confined to the stack and tables with `externref` elements.

Rust does not support reference types natively; there is no way to produce an import / export
that has `externref` as an argument or a return type. [`wasm-bindgen`] patches WASM if
`externref`s are enabled. This library strives to accomplish the same goal for generic
low-level WASM ABIs (`wasm-bindgen` is specialized for browser hosts).

## `externref` use cases

Since `externref`s are completely opaque from the module perspective, the only way to use
them is to send an `externref` back to the host as an argument of an imported function.
(Depending on the function semantics, the call may or may not consume the `externref`
and may or may not modify the underlying data; this is not reflected
by the WASM function signature.) An `externref` cannot be dereferenced by the module,
thus, the module cannot directly access or modify the data behind the reference. Indeed,
the module cannot even be sure which kind of data is being referenced.

It may seem that this limits `externref` utility significantly,
but `externref`s can still be useful, e.g. to model [capability-based security] tokens
or resource handles in the host environment. Another potential use case is encapsulating
complex data that would be impractical to transfer across the WASM API boundary
(especially if the data shape may evolve over time), and/or if interactions with data
must be restricted from the module side.

## Usage

Add this to your `Crate.toml`:

```toml
[dependencies]
externref = "0.1.0"
```

1. Use `Resource`s as arguments / return results for imported and/or exported functions
  in a WASM module in place of `externref`s. Reference args (including mutable references)
  and the `Option<_>` wrapper are supported as well.
2. Add the `#[externref]` proc macro on the imported / exported functions.
3. Transform the generated WASM module with the module processor
  from the corresponding module of the crate.

As an alternative for the final step, there is a [CLI app](crates/cli)
that can process WASM modules with slightly less fine-grained control.

> **Important.** The processor should run before WASM optimization tools such as
> `wasm-opt` from binaryen.

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
[capability-based security]: https://en.wikipedia.org/wiki/Capability-based_security
