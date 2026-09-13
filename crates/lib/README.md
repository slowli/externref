# Low-Cost Reference Type Shims For WASM Modules

[![CI](https://github.com/slowli/externref/actions/workflows/ci.yml/badge.svg)](https://github.com/slowli/externref/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%2FApache--2.0-blue)](https://github.com/slowli/externref#license)
![rust 1.85+ required](https://img.shields.io/badge/rust-1.85+-blue.svg?label=Required%20Rust)
![no_std supported](https://img.shields.io/badge/no__std-tested-green.svg)

**Documentation:** [![Docs.rs](https://docs.rs/externref/badge.svg)](https://docs.rs/externref/)
[![crate docs (main)](https://img.shields.io/badge/main-yellow.svg?label=docs)](https://slowli.github.io/externref/crates/externref/)
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
externref = "0.3.0"
```

1. Use `Resource`s as arguments / return results for imported and/or exported functions
  in a WASM module in place of `externref`s. Reference args (including mutable references)
  and the `Option<_>` wrapper are supported as well.
2. Add the `#[externref]` proc macro on the imported / exported functions.
3. Transform the generated WASM module with the module processor
  from the corresponding module of the crate.

As an alternative for the final step, there is a [CLI app](../cli)
that can process WASM modules with slightly less fine-grained control.

> **Important.** The processor should run before WASM optimization tools such as
> `wasm-opt` from binaryen.

### Reference nullability

The Rust argument or return type determines nullability in the processed WASM signature:

| Rust type | WASM type |
| --- | --- |
| `Resource<T>` | `(ref extern)` |
| `&Resource<T>` | `(ref extern)` |
| `&mut Resource<T>` | `(ref extern)` |
| `Option<Resource<T>>` | `(ref null extern)` |
| `Option<&Resource<T>>` | `(ref null extern)` |
| `Option<&mut Resource<T>>` | `(ref null extern)` |

The same rules apply to `ResourceCopy` and resource type aliases marked with `#[resource]`.
The `Option` wrapper must be visible in the signature for the macro to recognize it.
No separate nullability attribute is needed.

For example, the JS string `cast` builtin requires a nullable parameter and a non-null result:

```rust,no_run
use externref::{externref, Resource};

#[externref(stubs)]
#[link(wasm_import_module = "wasm:js-string")]
unsafe extern "C" {
    fn cast(value: Option<&Resource<()>>) -> Resource<()>;
}
```

An existing resource can be passed as `cast(Some(&resource))`. Use `Option` in an import
signature whenever the host requires a nullable WASM type, even if the host rejects null
values at runtime, as JS string builtins do.

The Rust representation and resource cleanup are unchanged. The internal table remains
nullable, and the processor inserts `ref.as_non_null` checks where references leave it
through a non-null interface. Nullable resources already map host nulls to `None` at runtime;
no additional Rust wrapper type is required.

Use the matching updated processor or CLI: older processors do not read the supplementary
non-null metadata. The WASM engine must support non-null reference types. When optimizing
with Binaryen, enable GC support with `wasm-opt --enable-gc`; otherwise, it can lower
non-null signatures to nullable ones and invalidate builtin imports.

### Limitations

If you compile WASM without compilation optimizations, you might get "incorrectly placed externref guard" errors during WASM processing.
Currently, the only workaround is to switch off some debug info for the compiled WASM module, e.g. using a workspace manifest:

```toml,no_sync
[profile.dev.package.your-wasm-module]
debug = 1 # or "limited" if you're targeting MSRV 1.71+
```

These errors shouldn't occur if WASM is compiled in the release mode.

### Examples

Using the `#[externref]` macro and `Resource`s in WASM-targeting code:

<!-- ANCHOR: example -->
```rust
use externref::{externref, Resource};

// Two marker types for different resources.
pub struct Arena(());
pub struct Bytes(());

#[cfg(target_arch = "wasm32")]
#[externref]
#[link(wasm_import_module = "arena")]
extern "C" {
    // This import will have signature `((ref extern), i32) -> externref`
    // on host.
    fn alloc(arena: &Resource<Arena>, size: usize) 
        -> Option<Resource<Bytes>>;
}

// Fallback for non-WASM targets.
#[cfg(not(target_arch = "wasm32"))]
unsafe fn alloc(_: &Resource<Arena>, _: usize) 
    -> Option<Resource<Bytes>> { None }

// This export will have signature `((ref extern)) -> ()` on host.
#[externref]
#[unsafe(export_name = "test_export")]
pub extern "C" fn test_export(arena: &Resource<Arena>) {
    let bytes = unsafe { alloc(arena, 42) }.expect("cannot allocate");
    // Do something with `bytes`...
}
```
<!-- ANCHOR_END: example -->

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
