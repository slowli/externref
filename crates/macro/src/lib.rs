//! Procedural macro for [`externref`].
//!
//! This macro wraps imported or exported functions with `Resource` args / return type
//! doing all heavy lifting to prepare these functions for usage with `externref`s.
//! Note that it is still necessary to post-process the module with [`externref-processor`].
//!
//! See `externref` docs for more details and examples of usage.
//!
//! [`externref`]: https://docs.rs/externref
//! [`externref-processor`]: https://docs.rs/externref-processor

#![recursion_limit = "128"]
// Linter settings.
#![warn(missing_debug_implementations, missing_docs, bare_trait_objects)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::must_use_candidate, clippy::module_name_repetitions)]

extern crate proc_macro;

use proc_macro::TokenStream;
use syn::Item;

mod externref;

use crate::externref::{for_export, for_foreign_module};

/// Prepares imported functions or an exported function with `Resource` args and/or return type.
///
/// This attribute must be placed on an `extern "C" { ... }` block or an `extern "C" fn`.
/// If placed on block, all enclosed functions with `Resource` args / return type will be
/// wrapped.
#[proc_macro_attribute]
pub fn externref(_attr: TokenStream, input: TokenStream) -> TokenStream {
    const MSG: &str = "Unsupported item; only `extern \"C\" {}` modules and `extern \"C\" fn ...` \
        exports are supported";

    let output = match syn::parse::<Item>(input) {
        Ok(Item::ForeignMod(mut module)) => for_foreign_module(&mut module),
        Ok(Item::Fn(mut function)) => for_export(&mut function),
        Ok(other_item) => {
            return darling::Error::custom(MSG)
                .with_span(&other_item)
                .write_errors()
                .into()
        }
        Err(err) => return err.into_compile_error().into(),
    };
    output.into()
}
