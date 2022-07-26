use proc_macro2::TokenStream;
use quote::{quote, ToTokens};
use syn::{
    spanned::Spanned, Attribute, FnArg, ForeignItem, Ident, ItemFn, ItemForeignMod, Lit, LitStr,
    Meta, MetaList, NestedMeta, PatType, PathArguments, ReturnType, Signature, Type, TypePath,
};

use std::collections::HashMap;

fn check_abi(abi_name: Option<&LitStr>, root_span: &impl Spanned) -> darling::Result<()> {
    let abi_name = abi_name.ok_or_else(|| {
        darling::Error::custom("Exported function must be marked with `extern \"C\"`")
            .with_span(root_span)
    })?;
    if abi_name.value() != "C" {
        let msg = format!(
            "Unexpected ABI {} for exported function; expected `C`",
            abi_name.value()
        );
        return Err(darling::Error::custom(msg).with_span(&abi_name));
    }
    Ok(())
}

fn attr_string(attrs: &[Attribute], name: &str) -> darling::Result<Option<String>> {
    let attr = attrs.iter().find(|attr| attr.path.is_ident(name));
    let attr = if let Some(attr) = attr {
        attr
    } else {
        return Ok(None);
    };

    let attr_value = if let Meta::NameValue(nv) = attr.parse_meta()? {
        nv.lit
    } else {
        let msg = format!(
            "Unexpected `{}` attribute format; expected a name-value pair",
            name
        );
        return Err(darling::Error::custom(msg).with_span(attr));
    };
    if let Lit::Str(str) = attr_value {
        Ok(Some(str.value()))
    } else {
        let msg = format!("Unexpected `{}` value; expected a string", name);
        Err(darling::Error::custom(msg).with_span(attr))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ArgKind {
    Owned,
    Ref,
    MutRef,
}

impl ArgKind {
    fn is_resource(ty: &TypePath) -> bool {
        ty.path.segments.last().map_or(false, |segment| {
            segment.ident == "Resource"
                && matches!(
                    &segment.arguments,
                    PathArguments::AngleBracketed(args) if args.args.len() == 1
                )
        })
    }

    fn from_type(ty: &Type) -> Option<Self> {
        match ty {
            Type::Path(path) if Self::is_resource(path) => Some(Self::Owned),
            Type::Reference(reference) => {
                if let Type::Path(path) = reference.elem.as_ref() {
                    if Self::is_resource(path) {
                        return Some(if reference.mutability.is_some() {
                            Self::MutRef
                        } else {
                            Self::Ref
                        });
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn initialize_for_export(self, arg: &Ident) -> TokenStream {
        let owned = quote!(unsafe { externref::Resource::new(#arg) });
        match self {
            Self::Owned => owned,
            Self::Ref => quote!(&#owned),
            Self::MutRef => quote!(&mut #owned),
        }
    }

    fn prepare_for_import(self, arg: &Ident) -> TokenStream {
        match self {
            Self::Ref | Self::MutRef => quote!(externref::Resource::as_raw(#arg)),
            Self::Owned => quote!(externref::Resource::into_raw(#arg)),
        }
    }
}

#[derive(Debug)]
struct Function {
    name: String,
    arg_count: usize,
    resource_args: HashMap<usize, ArgKind>,
    // `None` if the function does not have a return type
    resource_return_type: Option<bool>,
}

impl Function {
    fn new(function: &ItemFn) -> darling::Result<Self> {
        let abi_name = function.sig.abi.as_ref().and_then(|abi| abi.name.as_ref());
        check_abi(abi_name, &function.sig)?;

        if let Some(variadic) = &function.sig.variadic {
            let msg = "Variadic functions are not supported";
            return Err(darling::Error::custom(msg).with_span(variadic));
        }
        let export_name = attr_string(&function.attrs, "export_name")?;
        Ok(Self::from_sig(&function.sig, export_name))
    }

    fn from_sig(sig: &Signature, name_override: Option<String>) -> Self {
        let resource_args = sig.inputs.iter().enumerate().filter_map(|(i, arg)| {
            if let FnArg::Typed(PatType { ty, .. }) = arg {
                return ArgKind::from_type(ty).map(|kind| (i, kind));
            }
            None
        });
        let resource_return_type = match &sig.output {
            ReturnType::Type(_, ty) => {
                let is_resource =
                    ArgKind::from_type(ty).map_or(false, |kind| kind == ArgKind::Owned);
                Some(is_resource)
            }
            ReturnType::Default => None,
        };

        Self {
            name: name_override.unwrap_or_else(|| sig.ident.to_string()),
            arg_count: sig.inputs.len(),
            resource_args: resource_args.collect(),
            resource_return_type,
        }
    }

    fn needs_declaring(&self) -> bool {
        !self.resource_args.is_empty() || self.resource_return_type == Some(true)
    }

    fn declare(&self, module_name: Option<&str>) -> impl ToTokens {
        let name = &self.name;
        let kind = if let Some(module_name) = module_name {
            quote!(externref::signature::FunctionKind::Import(#module_name))
        } else {
            quote!(externref::signature::FunctionKind::Export)
        };
        let externrefs = self.create_externrefs();

        quote! {
            externref::declare_function!(externref::signature::Function {
                kind: #kind,
                name: #name,
                externrefs: #externrefs,
            });
        }
    }

    fn wrap_export(&self, raw: &ItemFn, export_name: Option<Attribute>) -> impl ToTokens {
        let export_name = export_name.unwrap_or_else(|| {
            let name = raw.sig.ident.to_string();
            syn::parse_quote!(#[export_name = #name])
        });
        let mut export_sig = raw.sig.clone();
        export_sig.abi = Some(syn::parse_quote!(extern "C"));
        export_sig.ident = Ident::new("__externref_export", export_sig.ident.span());

        let mut args = Vec::with_capacity(export_sig.inputs.len());
        for (i, arg) in export_sig.inputs.iter_mut().enumerate() {
            if let FnArg::Typed(typed_arg) = arg {
                let arg = Ident::new(&format!("__arg{}", i), typed_arg.pat.span());
                typed_arg.pat = Box::new(syn::parse_quote!(#arg));

                if let Some(kind) = self.resource_args.get(&i) {
                    typed_arg.ty = Box::new(syn::parse_quote!(usize));
                    args.push(kind.initialize_for_export(&arg));
                } else {
                    args.push(quote!(#arg));
                }
            }
        }

        let original_name = &raw.sig.ident;
        let delegation = quote!(#original_name(#(#args,)*));
        let delegation = match self.resource_return_type {
            Some(true) => {
                export_sig.output = syn::parse_quote!(-> usize);
                quote! {
                    let original_output = #delegation;
                    unsafe { externref::Resource::into_raw(original_output) }
                }
            }
            Some(false) => delegation,
            None => quote!(#delegation;),
        };

        quote! {
            const _: () = {
                #[no_mangle]
                #export_name
                #export_sig {
                    #delegation
                }
            };
        }
    }

    fn wrap_import(&self, mut sig: Signature) -> (TokenStream, Ident) {
        sig.unsafety = Some(syn::parse_quote!(unsafe));
        let new_ident = format!("__externref_{}", sig.ident);
        let new_ident = Ident::new(&new_ident, sig.ident.span());

        let mut args = Vec::with_capacity(sig.inputs.len());
        for (i, arg) in sig.inputs.iter_mut().enumerate() {
            if let FnArg::Typed(typed_arg) = arg {
                let arg = Ident::new(&format!("__arg{}", i), typed_arg.pat.span());
                typed_arg.pat = Box::new(syn::parse_quote!(#arg));

                if let Some(kind) = self.resource_args.get(&i) {
                    args.push(kind.prepare_for_import(&arg));
                } else {
                    args.push(quote!(#arg));
                }
            }
        }

        let delegation = quote!(#new_ident(#(#args,)*));
        let delegation = match self.resource_return_type {
            Some(true) => {
                quote! {
                    let original_output = #delegation;
                    externref::Resource::new(original_output)
                }
            }
            Some(false) => delegation,
            None => quote!(#delegation;),
        };

        (quote!(#sig { #delegation }), new_ident)
    }

    fn create_externrefs(&self) -> impl ToTokens {
        let args_and_return_type_count = if self.resource_return_type.is_some() {
            self.arg_count + 1
        } else {
            self.arg_count
        };
        let bytes = (args_and_return_type_count + 7) / 8;

        let maybe_ret_idx = self.resource_return_type.and_then(|is_resource| {
            if is_resource {
                Some(self.arg_count)
            } else {
                None
            }
        });

        let set_bits = self.resource_args.keys().copied();
        #[cfg(test)] // sort keys in deterministic order for testing
        let set_bits = {
            let mut sorted: Vec<_> = set_bits.collect();
            sorted.sort_unstable();
            sorted.into_iter()
        };
        let set_bits = set_bits.chain(maybe_ret_idx);
        let set_bits = set_bits.map(|idx| quote!(.with_set_bit(#idx)));

        quote! {
            externref::signature::BitSlice::builder::<#bytes>(#args_and_return_type_count)
                #(#set_bits)*
                .build()
        }
    }
}

pub(crate) fn for_export(function: &mut ItemFn) -> TokenStream {
    let parsed_function = match Function::new(function) {
        Ok(function) => function,
        Err(err) => return err.write_errors(),
    };
    let (declaration, export) = if parsed_function.needs_declaring() {
        // "Un-export" the function by removing the relevant attributes.
        function.sig.abi = None;
        let attr_idx = function.attrs.iter().enumerate().find_map(|(idx, attr)| {
            if attr.path.is_ident("export_name") {
                Some(idx)
            } else {
                None
            }
        });
        let export_name_attr = attr_idx.map(|idx| function.attrs.remove(idx));

        let export = parsed_function.wrap_export(function, export_name_attr);
        (Some(parsed_function.declare(None)), Some(export))
    } else {
        (None, None)
    };

    quote! {
        #function
        #export
        #declaration
    }
}

#[derive(Debug)]
struct Imports {
    module_name: String,
    functions: Vec<(Function, TokenStream)>,
}

impl Imports {
    fn new(module: &mut ItemForeignMod) -> darling::Result<Self> {
        const NO_ATTR_MSG: &str = "#[link(wasm_import_module = \"..\")] must be specified \
            on the foreign module";

        check_abi(module.abi.name.as_ref(), &module.abi)?;

        let link_attr = module.attrs.iter().find(|attr| attr.path.is_ident("link"));
        let link_attr =
            link_attr.ok_or_else(|| darling::Error::custom(NO_ATTR_MSG).with_span(&module))?;
        let link_meta = link_attr.parse_meta()?;

        let module_name = if let Meta::List(MetaList { nested, .. }) = &link_meta {
            nested.iter().find_map(|nested_meta| match nested_meta {
                NestedMeta::Meta(Meta::NameValue(nv)) if nv.path.is_ident("wasm_import_module") => {
                    Some(&nv.lit)
                }
                _ => None,
            })
        } else {
            let msg = "Unexpected contents of `#[link(..)]` attr (expected a list)";
            return Err(darling::Error::custom(msg).with_span(link_attr));
        };

        let module_name =
            module_name.ok_or_else(|| darling::Error::custom(NO_ATTR_MSG).with_span(link_attr))?;
        let module_name = if let Lit::Str(str) = module_name {
            str.value()
        } else {
            let msg = "Unexpected WASM module name format (expected a string)";
            return Err(darling::Error::custom(msg).with_span(module_name));
        };

        let mut functions = Vec::with_capacity(module.items.len());
        for item in &mut module.items {
            if let ForeignItem::Fn(fn_item) = item {
                let link_name = attr_string(&fn_item.attrs, "link_name")?;
                let has_link_name = link_name.is_some();
                let function = Function::from_sig(&fn_item.sig, link_name);
                if !function.needs_declaring() {
                    continue;
                }

                let (wrapper, new_ident) = function.wrap_import(fn_item.sig.clone());
                if !has_link_name {
                    // Add `#[link_name = ".."]` since the function is renamed.
                    let name = fn_item.sig.ident.to_string();
                    fn_item.attrs.push(syn::parse_quote!(#[link_name = #name]));
                }
                fn_item.sig.ident = new_ident;

                // Change function signature to use `usize`s in place of `Resource`s.
                for (i, arg) in fn_item.sig.inputs.iter_mut().enumerate() {
                    if function.resource_args.contains_key(&i) {
                        if let FnArg::Typed(typed_arg) = arg {
                            typed_arg.ty = Box::new(syn::parse_quote!(usize));
                        }
                    }
                }
                if function.resource_return_type == Some(true) {
                    fn_item.sig.output = syn::parse_quote!(-> usize);
                }

                functions.push((function, wrapper));
            }
        }

        Ok(Self {
            module_name,
            functions,
        })
    }

    fn declarations(&self) -> impl ToTokens {
        let function_declarations = self
            .functions
            .iter()
            .map(|(function, _)| function.declare(Some(&self.module_name)));
        quote!(#(#function_declarations)*)
    }

    fn wrappers(&self) -> impl ToTokens {
        let wrappers = self.functions.iter().map(|(_, wrapper)| wrapper);
        quote!(#(#wrappers)*)
    }
}

pub(crate) fn for_foreign_module(module: &mut ItemForeignMod) -> TokenStream {
    let parsed_module = match Imports::new(module) {
        Ok(module) => module,
        Err(err) => return err.write_errors(),
    };
    let declarations = parsed_module.declarations();
    let wrappers = parsed_module.wrappers();
    quote! {
        #module
        #declarations
        #wrappers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declaring_signature_for_export() {
        let export_fn: ItemFn = syn::parse_quote! {
            pub extern "C" fn test_export(
                sender: &mut Resource<Sender>,
                buffer: Resource<Buffer>,
                some_ptr: *const u8,
            ) {
                // does nothing
            }
        };
        let parsed = Function::new(&export_fn).unwrap();
        assert!(parsed.needs_declaring());

        let declaration = parsed.declare(None);
        let declaration: syn::Item = syn::parse_quote!(#declaration);
        let expected: syn::Item = syn::parse_quote! {
            externref::declare_function!(externref::signature::Function {
                kind: externref::signature::FunctionKind::Export,
                name: "test_export",
                externrefs: externref::signature::BitSlice::builder::<1usize>(3usize)
                    .with_set_bit(0usize)
                    .with_set_bit(1usize)
                    .build(),
            });
        };
        assert_eq!(declaration, expected, "{}", quote!(#declaration));
    }

    #[test]
    fn transforming_export() {
        let export_fn: ItemFn = syn::parse_quote! {
            pub extern "C" fn test_export(
                sender: &mut Resource<Sender>,
                buffer: Resource<Buffer>,
                some_ptr: *const u8,
            ) {
                // does nothing
            }
        };
        let parsed = Function::new(&export_fn).unwrap();
        assert_eq!(parsed.resource_args.len(), 2);
        assert_eq!(parsed.resource_args[&0], ArgKind::MutRef);
        assert_eq!(parsed.resource_args[&1], ArgKind::Owned);
        assert_eq!(parsed.resource_return_type, None);

        let wrapper = parsed.wrap_export(&export_fn, None);
        let wrapper: syn::Item = syn::parse_quote!(#wrapper);
        let expected: syn::Item = syn::parse_quote! {
            const _: () = {
                #[no_mangle]
                #[export_name = "test_export"]
                extern "C" fn __externref_export(
                    __arg0: usize,
                    __arg1: usize,
                    __arg2: *const u8,
                ) {
                    test_export(
                        &mut unsafe { externref::Resource::new(__arg0) },
                        unsafe { externref::Resource::new(__arg1) },
                        __arg2,
                    );
                }
            };
        };
        assert_eq!(wrapper, expected, "{}", quote!(#wrapper));
    }

    #[test]
    fn wrapper_for_import() {
        let sig: Signature = syn::parse_quote! {
            fn send_message(
                sender: &Resource<Sender>,
                message_ptr: *const u8,
                message_len: usize,
            ) -> Resource<Bytes>
        };
        let parsed = Function::from_sig(&sig, None);

        let (wrapper, ident) = parsed.wrap_import(sig);
        assert_eq!(ident, "__externref_send_message");

        let wrapper: ItemFn = syn::parse_quote!(#wrapper);
        let expected: ItemFn = syn::parse_quote! {
            unsafe fn send_message(
                __arg0: &Resource<Sender>,
                __arg1: *const u8,
                __arg2: usize,
            ) -> Resource<Bytes> {
                let original_output = __externref_send_message(
                    externref::Resource::as_raw(__arg0),
                    __arg1,
                    __arg2,
                );
                externref::Resource::new(original_output)
            }
        };
        assert_eq!(wrapper, expected, "{}", quote!(#wrapper));
    }

    #[test]
    fn foreign_mod_transformation() {
        let mut foreign_mod: ItemForeignMod = syn::parse_quote! {
            #[link(wasm_import_module = "test")]
            extern "C" {
                fn send_message(
                    sender: &Resource<Sender>,
                    message_ptr: *const u8,
                    message_len: usize,
                ) -> Resource<Bytes>;
            }
        };
        Imports::new(&mut foreign_mod).unwrap();

        let expected: ItemForeignMod = syn::parse_quote! {
            #[link(wasm_import_module = "test")]
            extern "C" {
                #[link_name = "send_message"]
                fn __externref_send_message(
                    sender: usize,
                    message_ptr: *const u8,
                    message_len: usize,
                ) -> usize;
            }
        };
        assert_eq!(foreign_mod, expected, "{}", quote!(#foreign_mod));
    }
}
