# Introduction

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

`externref` is available as a [library](library.md) and a [CLI app](cli).
These two are complementary: the library is used on the WASM (guest) side, and the CLI app
can be used for post-processing of the generated WASM modules. (Such post-processing is mandatory,
but it can be performed with the library as well.)

[reference type]: https://webassembly.github.io/spec/core/syntax/types.html#reference-types
[`wasm-bindgen`]: https://crates.io/crates/wasm-bindgen
[capability-based security]: https://en.wikipedia.org/wiki/Capability-based_security
