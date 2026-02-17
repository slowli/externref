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
#![doc(html_root_url = "https://docs.rs/externref-macro/0.3.0-beta.1")]
// Linter settings.
#![warn(missing_debug_implementations, missing_docs, bare_trait_objects)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::must_use_candidate, clippy::module_name_repetitions)]

extern crate proc_macro;

use proc_macro::TokenStream;
use syn::{
    Item, Path,
    parse::{Error as SynError, Parser},
};

mod externref;

use crate::externref::{for_export, for_foreign_module};

#[derive(Default)]
struct ExternrefAttrs {
    crate_path: Option<Path>,
}

impl ExternrefAttrs {
    fn parse(tokens: TokenStream) -> syn::Result<Self> {
        let mut attrs = Self::default();
        if tokens.is_empty() {
            return Ok(attrs);
        }

        let parser = syn::meta::parser(|meta| {
            if meta.path.is_ident("crate") {
                let value = meta.value()?;
                attrs.crate_path = Some(if let Ok(path_str) = value.parse::<syn::LitStr>() {
                    path_str.parse()?
                } else {
                    value.parse()?
                });
                Ok(())
            } else {
                Err(meta.error("unsupported attribute"))
            }
        });
        parser.parse(tokens)?;
        Ok(attrs)
    }

    fn crate_path(&self) -> Path {
        self.crate_path
            .clone()
            .unwrap_or_else(|| syn::parse_quote!(externref))
    }
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
/// - `Resource<..>`, `&Resource<..>`, `&mut Resource<..>` with one or two type args
/// - `ResourceCopy<..>` and its references
/// - `Option<_>` of the above six variations
///
/// For complex cases, resource detection can be controlled with a `#[resource]` attribute.
/// This attribute can be placed on a function arg or on the function itself (in which case it corresponds
/// to the return type; attributes cannot be placed on the return type directly).
///
/// - `#[resource]`, `#[resource = true]` or `#[resource(true)]` mark an arg / return type as a resource.
/// - `#[resource = false]` or `#[resource(false)]` mark an arg / return type as a non-resource.
#[proc_macro_attribute]
pub fn externref(attr: TokenStream, input: TokenStream) -> TokenStream {
    const MSG: &str = "Unsupported item; only `extern \"C\" {}` modules and `extern \"C\" fn ...` \
        exports are supported";

    let attrs = match ExternrefAttrs::parse(attr) {
        Ok(attrs) => attrs,
        Err(err) => return err.into_compile_error().into(),
    };

    let output = match syn::parse::<Item>(input) {
        Ok(Item::ForeignMod(mut module)) => for_foreign_module(&mut module, &attrs),
        Ok(Item::Fn(mut function)) => for_export(&mut function, &attrs),
        Ok(other) => {
            return SynError::new_spanned(other, MSG)
                .into_compile_error()
                .into();
        }
        Err(err) => return err.into_compile_error().into(),
    };
    output.into()
}
