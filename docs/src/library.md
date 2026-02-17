# Using Library

Add this to your `Crate.toml`:

```toml
[dependencies]
externref = "0.3.0-beta.1"
```

See [the library docs](crates/externref) for detailed description of its API.

## General workflow

The basic approach is as follows:

1. Use [`Resource`]s as arguments / return results for imported and/or exported functions
   in a WASM module in place of `externref`s . Reference args (including mutable references)
   and the `Option<_>` wrapper are supported as well.
2. Add the `#[externref]` proc macro on the imported / exported functions.
3. Post-process the generated WASM module with the processor. This can be possible 

`Resource`s support primitive downcasting and upcasting with `Resource<()>` signalling
a generic resource. Downcasting is *unchecked*; it is up to the `Resource` users to
define a way to check the resource kind dynamically if necessary. One possible approach
for this is defining a WASM import `fn(&Resource<()>) -> Kind`, where `Kind` is the encoded
kind of the supplied resource, such as `i32`.

> [!NOTE]
>
> `Resource` is essentially a smart pointer. Correspondingly, its `Eq` and `Hash` trait implementations
> treat it as such (i.e., two resources are equal if they point to the same externref).

> [!NOTE]
> 
> `Resource` implements the RAII pattern, in which it releases the associated `externref` on drop.
> Correspondingly, `Resource` does not implement `Clone` / `Copy`. To clone resources, you may want to
> wrap it in a `Rc` / `Arc`, or to use [copyable resources](#copyable-resources).

## Basic example

The code sample below demonstrates the basic usage of the `#[externref]` proc macro
with 2 resource types.

{{#include ../../crates/lib/README.md:example}}

## Copyable resources

[`ResourceCopy`] is a variation of `Resource` that implements `Clone` / `Copy`. As a trade-off, it **does not**
implement any resource management.

<!-- FIXME: example -->

[`Resource`]: crates/externref/struct.Resource.html
[`ResourceCopy`]: crates/externref/type.ResourceCopy.html
