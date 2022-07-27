//! Low-cost [reference type] shims for WASM modules.
//!
//! Reference type (aka `externref` or `anyref`) is an opaque reference made available to
//! a WASM module by the host environment. Such references cannot be forged in the WASM code
//! and can be associated with arbitrary host data, thus making them a good alternative to
//! ad-hoc `usize` handles etc. References cannot be stored in WASM linear memory; they are
//! thus confined to the stack and tables with `externref` elements.
//!
//! Rust does not support reference types natively; there is no way to produce an import / export
//! that has `externref` as an argument or a return type. [`wasm-bindgen`] patches WASM if
//! `externref`s are enabled. This library strives to accomplish the same goal for generic
//! low-level WASM ABIs (`wasm-bindgen` is specialized for browser hosts).
//!
//! # Workflow
//!
//! 1. Declare [`Resource`]s as arguments / return results for imported and/or exported functions
//!   in a WASM module. Reference args (including mutable references) are supported as well.
//! 2. Add the `#[externref]` proc macro on the imported / exported functions.
//! 3. Post-process the generated WASM module with [`externref-processor`].
//!
//! # How it works
//!
//! The [`externref` macro](macro@externref) detects `Resource` args / return types
//! for imported and exported functions. All `Resource` args or return types are replaced
//! with `usize`s and a wrapper function is added that performs the necessary transform
//! from / to `usize`.
//! Additionally, a function signature describing where `Resource` args are located
//! is recorded in a WASM custom section.
//!
//! To handle `usize` (~`i32` in WASM) <-> `externref` conversions, `Resource` methods
//! reference imports from a surrogate module:
//!
//! - [`Resource::new()`] ("real" signature `fn(externref) -> usize`) stores a reference
//!   into a table and returns the table index. The index is what is actually stored within
//!   the `Resource`, meaning that `Resource`s can be easily placed on heap.
//! - [`Resource::as_raw()`] ("real" signature `fn(usize) -> externref`) gets a reference
//!   given the table index.
//! - [`Resource::drop()`] ("real" signature `fn(usize)`) removes the reference from the table.
//!
//! Since `externref` is not presentable in Rust, `externref`s in `new()` / `as_raw()`
//! are replaced with `usize`s. They are patched back by the post-processor:
//!
//! - Imports referenced by `new()` / `as_raw()` / `drop()` are replaced with local WASM functions.
//!   `as_raw()` and `drop()` functions are trivial; `new()` is less so (we don't want to
//!   excessively grow the `externref`s table, thus we search for null refs among its existing
//!   elements first, and only then grow the table).
//! - Patching changes function types, and as a result types of some locals.
//!   This is OK because the post-processor also changes the signatures of affected
//!   imported / exported functions. The success relies on the fact that
//!   it only makes sense to call `new()` immediately after receiving the reference from host,
//!   and `as_raw()` immediately before passing the reference to host. `drop()` can be called
//!   anywhere, but it does not have its type changed.
//!
//! # Examples
//!
//! ```no_run
//! use externref::{externref, Resource};
//!
//! // Two marker types for different resources.
//! pub struct Sender(());
//! pub struct Bytes(());
//!
//! #[externref]
//! #[link(wasm_import_module = "test")]
//! extern "C" {
//!     // This import will have signature `(externref, i32, i32) -> externref`
//!     // on host.
//!     fn send_message(
//!         sender: &Resource<Sender>,
//!         message_ptr: *const u8,
//!         message_len: usize,
//!     ) -> Resource<Bytes>;
//! }
//!
//! // This export will have signature `(externref)` on host.
//! #[externref]
//! #[export_name = "test_export"]
//! pub extern "C" fn test_export(sender: Resource<Sender>) {
//!     let messages: Vec<_> = ["test", "42", "some other string"]
//!         .into_iter()
//!         .map(|msg| {
//!             unsafe { send_message(&sender, msg.as_ptr(), msg.len()) }
//!         })
//!         .collect();
//!     // ...
//!     // All 4 resources are dropped when exiting the function.
//! }
//! ```
//!
//! [reference type]: https://webassembly.github.io/spec/core/syntax/types.html#reference-types
//! [`wasm-bindgen`]: https://crates.io/crates/wasm-bindgen
//! [`externref-processor`]: https://docs.rs/externref-processor

// Linter settings.
#![warn(missing_debug_implementations, missing_docs, bare_trait_objects)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(
    clippy::must_use_candidate,
    clippy::module_name_repetitions,
    clippy::inline_always
)]

use core::marker::PhantomData;

#[doc(hidden)]
pub mod signature;

#[cfg(feature = "macro")]
pub use externref_macro::externref;

/// `externref` surrogate.
#[derive(Debug)]
#[repr(transparent)]
pub struct ExternRef(usize);

/// Host resource exposed to WASM.
///
/// Internally, a resource is just an index into the `externref`s table; thus, it is completely
/// valid to store `Resource`s on heap (in a `Vec`, thread-local storage, etc.). The type param
/// can be used for type safety.
#[derive(Debug)]
pub struct Resource<T> {
    id: usize,
    _ty: PhantomData<fn(T)>,
}

impl<T> Resource<T> {
    /// Creates a new resource converting it from.
    ///
    /// # Safety
    ///
    /// This method must be called with an `externref` obtained from the host (as a return
    /// type for an imported function or an argument for an exported function); it is not
    /// a "real" `usize`. The proper use is ensured by the [`externref`] macro.
    /// Use this method manually only if you know what you are doing.
    #[inline(always)]
    pub unsafe fn new(id: ExternRef) -> Option<Self> {
        #[cfg(target_arch = "wasm32")]
        #[link(wasm_import_module = "externref")]
        extern "C" {
            #[link_name = "insert"]
            fn insert_externref(id: ExternRef) -> usize;
        }

        #[cfg(not(target_arch = "wasm32"))]
        #[allow(clippy::needless_pass_by_value)]
        unsafe fn insert_externref(id: ExternRef) -> usize {
            id.0
        }

        let id = insert_externref(id);
        if id == usize::MAX {
            None
        } else {
            Some(Self {
                id,
                _ty: PhantomData,
            })
        }
    }

    /// Obtains an `externref` from this resource.
    ///
    /// # Safety
    ///
    /// The returned value of this method must be passed as an `externref` to the host
    /// (as a return type of an exported function or an argument of the imported function);
    /// it is not a "real" `usize`. The proper use is ensured by the [`externref`] macro.
    /// Use this method manually only if you know what you are doing.
    #[inline(always)]
    pub unsafe fn raw(this: Option<&Self>) -> ExternRef {
        #[cfg(target_arch = "wasm32")]
        #[link(wasm_import_module = "externref")]
        extern "C" {
            #[link_name = "get"]
            fn get_externref(id: usize) -> ExternRef;
        }

        #[cfg(not(target_arch = "wasm32"))]
        unsafe fn get_externref(id: usize) -> ExternRef {
            ExternRef(id)
        }

        get_externref(this.map_or(usize::MAX, |resource| resource.id))
    }

    /// Obtains an `externref` from this resource and drops the resource.
    ///
    /// # Safety
    ///
    /// See [`Self::raw()`] for safety considerations.
    #[inline(always)]
    #[allow(clippy::needless_pass_by_value)]
    pub unsafe fn take_raw(this: Option<Self>) -> ExternRef {
        Self::raw(this.as_ref())
    }
}

/// Drops the `externref` associated with this resource.
impl<T> Drop for Resource<T> {
    #[inline(always)]
    fn drop(&mut self) {
        #[cfg(target_arch = "wasm32")]
        #[link(wasm_import_module = "externref")]
        extern "C" {
            #[link_name = "drop"]
            fn drop_externref(id: usize);
        }

        #[cfg(not(target_arch = "wasm32"))]
        unsafe fn drop_externref(_id: usize) {
            // Do nothing
        }

        unsafe { drop_externref(self.id) };
    }
}
