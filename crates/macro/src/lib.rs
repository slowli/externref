//! Procedural macro for [`externref`].

#![recursion_limit = "128"]

extern crate proc_macro;

use proc_macro::TokenStream;
use syn::Item;

mod externref;

use crate::externref::{for_export, for_foreign_module};

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
