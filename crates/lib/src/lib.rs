//! Low-cost [reference type] shims for WASM modules.
//!
//! Reference type (aka `externref` or `anyref`) is an opaque reference made available to
//! a WASM module by the host environment. Such references cannot be forged in the WASM code
//! and can be associated with arbitrary host data, thus making them a good alternative to
//! ad-hoc handles (e.g., numeric ones). References cannot be stored in WASM linear memory;
//! they are confined to the stack and tables with `externref` elements.
//!
//! Rust does not support reference types natively; there is no way to produce an import / export
//! that has `externref` as an argument or a return type. [`wasm-bindgen`] patches WASM if
//! `externref`s are enabled. This library strives to accomplish the same goal for generic
//! low-level WASM ABIs (`wasm-bindgen` is specialized for browser hosts).
//!
//! # `externref` use cases
//!
//! Since `externref`s are completely opaque from the module perspective, the only way to use
//! them is to send an `externref` back to the host as an argument of an imported function.
//! (Depending on the function semantics, the call may or may not consume the `externref`
//! and may or may not modify the underlying data; this is not reflected
//! by the WASM function signature.) An `externref` cannot be dereferenced by the module,
//! thus, the module cannot directly access or modify the data behind the reference. Indeed,
//! the module cannot even be sure which kind of data is being referenced.
//!
//! It may seem that this limits `externref` utility significantly,
//! but `externref`s can still be useful, e.g. to model [capability-based security] tokens
//! or resource handles in the host environment. Another potential use case is encapsulating
//! complex data that would be impractical to transfer across the WASM API boundary
//! (especially if the data shape may evolve over time), and/or if interactions with data
//! must be restricted from the module side.
//!
//! [capability-based security]: https://en.wikipedia.org/wiki/Capability-based_security
//!
//! # Usage
//!
//! 1. Use [`Resource`]s as arguments / return results for imported and/or exported functions
//!    in a WASM module in place of `externref`s . Reference args (including mutable references)
//!    and the `Option<_>` wrapper are supported as well.
//! 2. Add the `#[externref]` proc macro on the imported / exported functions.
//! 3. Post-process the generated WASM module with the [`processor`].
//!
//! `Resource`s support primitive downcasting and upcasting with `Resource<()>` signalling
//! a generic resource. Downcasting is *unchecked*; it is up to the `Resource` users to
//! define a way to check the resource kind dynamically if necessary. One possible approach
//! for this is defining a WASM import `fn(&Resource<()>) -> Kind`, where `Kind` is the encoded
//! kind of the supplied resource, such as `i32`.
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
//! To handle `usize` (~`i32` in WASM) <-> `externref` conversions, managing resources is performed
//! using 3 function imports from a surrogate module:
//!
//! - Creating a `Resource` ("real" signature `fn(externref) -> usize`) stores a reference
//!   into an `externref` table and returns the table index. The index is what is actually
//!   stored within the `Resource`, meaning that `Resource`s can be easily placed on heap.
//! - Getting a reference from a `Resource` ("real" signature `fn(usize) -> externref`)
//!   is an indexing operation for the `externref` table.
//! - [`Resource::drop()`] ("real" signature `fn(usize)`) removes the reference from the table.
//!
//! Real `externref`s are patched back to the imported / exported functions
//! by the WASM module post-processor:
//!
//! - Imports from a surrogate module referenced by `Resource` methods are replaced
//!   with local WASM functions. Functions for getting an `externref` from the table
//!   and dropping an `externref` are more or less trivial. Storing an `externref` is less so;
//!   we don't want to excessively grow the `externref`s table, thus we search for null refs
//!   among its existing elements first, and only grow the table if all existing table elements are
//!   occupied.
//! - Patching changes function types, and as a result types of some locals.
//!   This is OK because the post-processor also changes the signatures of affected
//!   imported / exported functions. The success relies on the fact that
//!   a reference is only stored *immediately* after receiving it from the host;
//!   likewise, a reference is only obtained *immediately* before passing it to the host.
//!   `Resource`s can be dropped anywhere, but the corresponding `externref` removal function
//!   does not need its type changed.
//!
//! [reference type]: https://webassembly.github.io/spec/core/syntax/types.html#reference-types
//! [`wasm-bindgen`]: https://crates.io/crates/wasm-bindgen
//!
//! ## Limitations
//!
//! With debug info enabled, surrogate `usize`s may be spilled onto the shadow stack (part
//! of the WASM linear memory used by `rustc` / LLVM when the main WASM stack is insufficient).
//! This makes replacing these surrogates with `externref`s much harder (essentially, it'd require symbolic execution
//! of the affected function to find out which locals should be replaced with `externref`s). This behavior should be detected
//! by the [processor], leading to "incorrectly placed externref guard" errors. Currently,
//! the only workaround is to [set debug info level](https://doc.rust-lang.org/cargo/reference/profiles.html#debug)
//! to `limited` or below for the compiled WASM module.
//!
//! # Crate features
//!
//! ## `std`
//!
//! *(Off by default)*
//!
//! Enables `std`-specific features, like [`Error`](std::error::Error) implementations for error types.
//!
//! ## `processor`
//!
//! *(Off by default)*
//!
//! Enables WASM module processing via the [`processor`] module. Requires the `std` feature.
//!
//! ## `tracing`
//!
//! *(Off by default)*
//!
//! Enables tracing during [module processing](processor) with the [`tracing`] facade.
//! Tracing events / spans mostly use `INFO` and `DEBUG` levels.
//!
//! [`tracing`]: https://docs.rs/tracing/
//!
//! # Examples
//!
//! Using the `#[externref]` macro and `Resource`s in WASM-targeting code:
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
//!
//!     // `Option`s are used to deal with null references. This function will have
//!     // `(externref) -> i32` signature.
//!     fn message_len(bytes: Option<&Resource<Bytes>>) -> usize;
//!     // This one has `() -> externref` signature.
//!     fn last_sender() -> Option<Resource<Sender>>;
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

#![cfg_attr(not(feature = "std"), no_std)]
// Documentation settings.
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc(html_root_url = "https://docs.rs/externref/0.3.0-beta.1")]
// Linter settings.
#![warn(missing_debug_implementations, missing_docs, bare_trait_objects)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(
    clippy::must_use_candidate,
    clippy::module_name_repetitions,
    clippy::inline_always
)]

use core::{
    alloc::Layout,
    fmt,
    hash::{Hash, Hasher},
    marker::PhantomData,
    mem, ptr,
};

#[cfg(feature = "macro")]
#[cfg_attr(docsrs, doc(cfg(feature = "macro")))]
pub use externref_macro::externref;

pub use crate::{
    error::{ReadError, ReadErrorKind},
    signature::{BitSlice, BitSliceBuilder, Function, FunctionKind},
};

mod error;
#[cfg(feature = "processor")]
#[cfg_attr(docsrs, doc(cfg(feature = "processor")))]
pub mod processor;
mod signature;

// Polyfill for `alloc` types.
mod alloc {
    #[cfg(not(feature = "std"))]
    extern crate alloc as std;

    pub(crate) use std::{format, string::String};
}

/// `externref` surrogate.
///
/// The post-processing logic replaces variables of this type with real `externref`s.
#[doc(hidden)] // should only be used by macro-generated code
#[repr(transparent)]
pub struct ExternRef(usize);

impl fmt::Debug for ExternRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("ExternRef").finish_non_exhaustive()
    }
}

impl ExternRef {
    /// Guard for imported function wrappers. The processor checks that each transformed function
    /// has this guard as the first instruction.
    ///
    /// # Safety
    ///
    /// This guard should only be inserted by the `externref` macro.
    #[inline(always)]
    pub unsafe fn guard() {
        #[cfg(target_arch = "wasm32")]
        #[link(wasm_import_module = "externref")]
        extern "C" {
            #[link_name = "guard"]
            fn guard();
        }

        #[cfg(target_arch = "wasm32")]
        guard();
    }
}

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

/// Host resource exposed to WASM.
///
/// Internally, a resource is just an index into the `externref`s table; thus, it is completely
/// valid to store `Resource`s on heap (in a `Vec`, thread-local storage, etc.). The type param
/// can be used for type safety.
#[derive(Debug)]
#[repr(C)]
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
    #[doc(hidden)] // should only be used by macro-generated code
    #[inline(always)]
    pub unsafe fn new(id: ExternRef) -> Option<Self> {
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

    #[doc(hidden)] // should only be used by macro-generated code
    #[inline(always)]
    pub unsafe fn new_non_null(id: ExternRef) -> Self {
        let id = insert_externref(id);
        assert!(
            id != usize::MAX,
            "Passed null `externref` as non-nullable arg"
        );
        Self {
            id,
            _ty: PhantomData,
        }
    }

    /// Obtains an `externref` from this resource.
    ///
    /// # Safety
    ///
    /// The returned value of this method must be passed as an `externref` to the host
    /// (as a return type of an exported function or an argument of the imported function);
    /// it is not a "real" `usize`. The proper use is ensured by the [`externref`] macro.
    #[doc(hidden)] // should only be used by macro-generated code
    #[inline(always)]
    pub unsafe fn raw(this: Option<&Self>) -> ExternRef {
        get_externref(match this {
            None => usize::MAX,
            Some(resource) => resource.id,
        })
    }

    /// Obtains an `externref` from this resource and drops the resource.
    #[doc(hidden)] // should only be used by macro-generated code
    #[inline(always)]
    #[allow(clippy::needless_pass_by_value)]
    pub unsafe fn take_raw(this: Option<Self>) -> ExternRef {
        get_externref(match this {
            None => usize::MAX,
            Some(resource) => resource.id,
        })
    }

    /// Upcasts this resource to a generic resource.
    pub fn upcast(self) -> Resource<()> {
        Resource {
            id: self.leak_id(),
            _ty: PhantomData,
        }
    }

    #[inline]
    fn leak_id(self) -> usize {
        let id = self.id;
        mem::forget(self);
        id
    }

    /// Upcasts a reference to this resource to a generic resource reference.
    pub fn upcast_ref(&self) -> &Resource<()> {
        debug_assert_eq!(Layout::new::<Self>(), Layout::new::<Resource<()>>());

        let ptr = ptr::from_ref(self).cast::<Resource<()>>();
        unsafe {
            // SAFETY: All resource types have identical alignment (thanks to `repr(C)`),
            // hence, casting among them is safe.
            &*ptr
        }
    }
}

impl Resource<()> {
    /// Downcasts this generic resource to a specific type.
    ///
    /// # Safety
    ///
    /// No checks are performed that the resource actually encapsulates what is meant
    /// by `Resource<T>`. It is up to the caller to check this beforehand (e.g., by calling
    /// a WASM import taking `&Resource<()>` and returning an app-specific resource kind).
    pub unsafe fn downcast_unchecked<T>(self) -> Resource<T> {
        Resource {
            id: self.leak_id(),
            _ty: PhantomData,
        }
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

/// Compares resources by their pointers, similar to [`ptr::eq()`].
impl<T> PartialEq for Resource<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T> Eq for Resource<T> {}

impl<T> Hash for Resource<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[cfg(doctest)]
doc_comment::doctest!("../README.md");
