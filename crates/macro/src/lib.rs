//! Procedural macro for [`externref`].
//!
//! This macro wraps imported or exported functions with `Resource` args / return type
//! doing all heavy lifting to prepare these functions for usage with `externref`s.
//! Note that it is necessary to post-process the module with the module processor provided
//! by the `externref` crate.
//!
//! See `externref` docs for more details and examples of usage.
//!
//! [`externref`]: https://docs.rs/externref

#![recursion_limit = "128"]
// Documentation settings.
#![doc(html_root_url = "https://docs.rs/externref-macro/0.1.0")]
// Linter settings.
#![warn(missing_debug_implementations, missing_docs, bare_trait_objects)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::must_use_candidate, clippy::module_name_repetitions)]

extern crate proc_macro;

use darling::FromMeta;
use proc_macro::TokenStream;
use syn::{
    parse::{Error as SynError, Parser},
    punctuated::Punctuated,
    Item, NestedMeta, Path, Token,
};

mod externref;

use crate::externref::{for_export, for_foreign_module};

#[derive(Debug, Default, FromMeta)]
struct ExternrefAttrs {
    #[darling(rename = "crate", default)]
    crate_path: Option<Path>,
}

impl ExternrefAttrs {
    fn crate_path(&self) -> Path {
        self.crate_path
            .clone()
            .unwrap_or_else(|| syn::parse_quote!(externref))
    }
}

fn parse_attr<T: FromMeta>(tokens: TokenStream) -> syn::Result<T> {
    let meta = Punctuated::<NestedMeta, Token![,]>::parse_terminated.parse(tokens)?;
    let meta: Vec<_> = meta.into_iter().collect();
    T::from_list(&meta).map_err(Into::into)
}

/// Prepares imported functions or an exported function with `Resource` args and/or return type.
///
/// # Inputs
///
/// This attribute must be placed on an `extern "C" { ... }` block or an `extern "C" fn`.
/// If placed on block, all enclosed functions with `Resource` args / return type will be
/// wrapped.
///
/// # Processing
///
/// The following arg / return types are recognized as resources:
///
/// - `Resource<_>`, `&Resource<_>`, `&mut Resource<_>`
/// - `Option<_>` of any of the above three variations
#[proc_macro_attribute]
pub fn externref(attr: TokenStream, input: TokenStream) -> TokenStream {
    const MSG: &str = "Unsupported item; only `extern \"C\" {}` modules and `extern \"C\" fn ...` \
        exports are supported";

    let attrs = match parse_attr::<ExternrefAttrs>(attr) {
        Ok(attrs) => attrs,
        Err(err) => return err.into_compile_error().into(),
    };

    let output = match syn::parse::<Item>(input) {
        Ok(Item::ForeignMod(mut module)) => for_foreign_module(&mut module, &attrs),
        Ok(Item::Fn(mut function)) => for_export(&mut function, &attrs),
        Ok(other) => {
            return SynError::new_spanned(other, MSG)
                .into_compile_error()
                .into()
        }
        Err(err) => return err.into_compile_error().into(),
    };
    output.into()
}
