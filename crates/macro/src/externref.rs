use proc_macro2::TokenStream;
use quote::{quote, ToTokens};
use syn::{
    parse::Error as SynError, punctuated::Punctuated, spanned::Spanned, Attribute, Expr, ExprLit,
    FnArg, ForeignItem, GenericArgument, Ident, ItemFn, ItemForeignMod, Lit, LitStr, Meta, PatType,
    Path, PathArguments, Signature, Token, Type, TypePath, Visibility,
};

use std::{collections::HashMap, mem};

use crate::ExternrefAttrs;

fn check_abi(
    target_name: &str,
    abi_name: Option<&LitStr>,
    root_span: &impl ToTokens,
) -> Result<(), SynError> {
    let abi_name = abi_name.ok_or_else(|| {
        let msg = format!("{target_name} must be marked with `extern \"C\"`");
        SynError::new_spanned(root_span, msg)
    })?;
    if abi_name.value() != "C" {
        let msg = format!(
            "Unexpected ABI {} for {target_name}; expected `C`",
            abi_name.value()
        );
        return Err(SynError::new(abi_name.span(), msg));
    }
    Ok(())
}

fn attr_expr(attrs: &[Attribute], name: &str) -> Result<Option<Expr>, SynError> {
    let attr = attrs.iter().find(|attr| attr.path().is_ident(name));
    let Some(attr) = attr else {
        return Ok(None);
    };

    let name_value = attr.meta.require_name_value()?;
    Ok(Some(name_value.value.clone()))
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SimpleResourceKind {
    Owned,
    Ref,
    MutRef,
}

impl SimpleResourceKind {
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
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ResourceKind {
    Simple(SimpleResourceKind),
    Option(SimpleResourceKind),
}

impl From<SimpleResourceKind> for ResourceKind {
    fn from(simple: SimpleResourceKind) -> Self {
        Self::Simple(simple)
    }
}

impl ResourceKind {
    fn parse_option(ty: &TypePath) -> Option<&Type> {
        let segment = ty.path.segments.last()?;
        if segment.ident == "Option" {
            if let PathArguments::AngleBracketed(args) = &segment.arguments {
                if args.args.len() == 1 {
                    if let GenericArgument::Type(ty) = args.args.first().unwrap() {
                        return Some(ty);
                    }
                }
            }
        }
        None
    }

    fn from_type(ty: &Type) -> Option<Self> {
        if let Some(kind) = SimpleResourceKind::from_type(ty) {
            return Some(kind.into());
        }

        if let Type::Path(path) = ty {
            Self::parse_option(path)
                .and_then(|inner_ty| SimpleResourceKind::from_type(inner_ty).map(Self::Option))
        } else {
            None
        }
    }

    fn simple_kind(self) -> SimpleResourceKind {
        match self {
            Self::Simple(simple) | Self::Option(simple) => simple,
        }
    }

    fn initialize_for_export(self, arg: &Ident, cr: &Path) -> TokenStream {
        let method_call = match self.simple_kind() {
            SimpleResourceKind::Owned => None,
            SimpleResourceKind::Ref => Some(quote!(.as_ref())),
            SimpleResourceKind::MutRef => Some(quote!(.as_mut())),
        };
        let unwrap = match self {
            Self::Option(_) => None,
            Self::Simple(_) => Some(quote!(.expect("null reference passed from host"))),
        };

        quote! {
            #cr::Resource::new(#arg) #method_call #unwrap
        }
    }

    fn prepare_for_import(self, arg: &Ident, cr: &Path) -> TokenStream {
        let arg = match self {
            Self::Simple(_) => quote!(core::option::Option::Some(#arg)),
            Self::Option(_) => quote!(#arg),
        };

        match self.simple_kind() {
            SimpleResourceKind::Ref | SimpleResourceKind::MutRef => {
                quote!(#cr::Resource::raw(#arg))
            }
            SimpleResourceKind::Owned => quote!(#cr::Resource::take_raw(#arg)),
        }
    }
}

#[derive(Debug, PartialEq)]
enum ReturnType {
    Default,
    NotResource,
    Resource(ResourceKind),
}

struct Function {
    name: Expr,
    arg_count: usize,
    resource_args: HashMap<usize, ResourceKind>,
    return_type: ReturnType,
    crate_path: Path,
}

impl Function {
    fn new(function: &ItemFn, attrs: &ExternrefAttrs) -> Result<Self, SynError> {
        let abi_name = function.sig.abi.as_ref().and_then(|abi| abi.name.as_ref());
        check_abi("exported function", abi_name, &function.sig)?;

        if let Some(variadic) = &function.sig.variadic {
            let msg = "Variadic functions are not supported";
            return Err(SynError::new_spanned(variadic, msg));
        }
        let export_name = attr_expr(&function.attrs, "export_name")?;
        Ok(Self::from_sig(&function.sig, export_name, attrs))
    }

    fn from_sig(sig: &Signature, name_override: Option<Expr>, attrs: &ExternrefAttrs) -> Self {
        let resource_args = sig.inputs.iter().enumerate().filter_map(|(i, arg)| {
            if let FnArg::Typed(PatType { ty, .. }) = arg {
                return ResourceKind::from_type(ty).map(|kind| (i, kind));
            }
            None
        });
        let return_type = match &sig.output {
            syn::ReturnType::Type(_, ty) => {
                ResourceKind::from_type(ty).map_or(ReturnType::NotResource, ReturnType::Resource)
            }
            syn::ReturnType::Default => ReturnType::Default,
        };
        let name = name_override.unwrap_or_else(|| {
            let str = sig.ident.to_string();
            syn::parse_quote!(#str)
        });

        Self {
            name,
            arg_count: sig.inputs.len(),
            resource_args: resource_args.collect(),
            return_type,
            crate_path: attrs.crate_path(),
        }
    }

    fn needs_declaring(&self) -> bool {
        !self.resource_args.is_empty() || matches!(self.return_type, ReturnType::Resource(_))
    }

    fn declare(&self, module_name: Option<&str>) -> impl ToTokens {
        let name = &self.name;
        let cr = &self.crate_path;
        let kind = if let Some(module_name) = module_name {
            quote!(#cr::FunctionKind::Import(#module_name))
        } else {
            quote!(#cr::FunctionKind::Export)
        };
        let externrefs = self.create_externrefs();

        quote! {
            #cr::declare_function!(#cr::Function {
                kind: #kind,
                name: #name,
                externrefs: #externrefs,
            });
        }
    }

    fn wrap_export(&self, raw: &ItemFn, export_name: Option<Attribute>) -> impl ToTokens {
        let cr = &self.crate_path;
        let export_name = export_name.unwrap_or_else(|| {
            let name = raw.sig.ident.to_string();
            syn::parse_quote!(#[export_name = #name])
        });
        let mut export_sig = raw.sig.clone();
        export_sig.abi = Some(syn::parse_quote!(extern "C"));
        export_sig.unsafety = Some(syn::parse_quote!(unsafe));
        export_sig.ident = Ident::new("__externref_export", export_sig.ident.span());

        let mut args = Vec::with_capacity(export_sig.inputs.len());
        for (i, arg) in export_sig.inputs.iter_mut().enumerate() {
            if let FnArg::Typed(typed_arg) = arg {
                let arg = Ident::new(&format!("__arg{i}"), typed_arg.pat.span());
                typed_arg.pat = Box::new(syn::parse_quote!(#arg));

                if let Some(kind) = self.resource_args.get(&i) {
                    typed_arg.ty = Box::new(syn::parse_quote!(#cr::ExternRef));
                    args.push(kind.initialize_for_export(&arg, cr));
                } else {
                    args.push(quote!(#arg));
                }
            }
        }

        let original_name = &raw.sig.ident;
        let delegation = quote!(#original_name(#(#args,)*));
        let delegation = match self.return_type {
            ReturnType::Resource(kind) => {
                export_sig.output = syn::parse_quote!(-> #cr::ExternRef);
                let output = Ident::new("__output", raw.sig.span());
                let conversion = kind.prepare_for_import(&output, cr);
                quote! {
                    let #output = #delegation;
                    #conversion
                }
            }
            ReturnType::NotResource => delegation,
            ReturnType::Default => quote!(#delegation;),
        };

        quote! {
            const _: () = {
                #export_name
                #export_sig {
                    #delegation
                }
            };
        }
    }

    fn wrap_import(&self, vis: &Visibility, mut sig: Signature) -> (TokenStream, Ident) {
        let cr = &self.crate_path;
        sig.unsafety = Some(syn::parse_quote!(unsafe));
        let new_ident = format!("__externref_{}", sig.ident);
        let new_ident = Ident::new(&new_ident, sig.ident.span());

        let mut args = Vec::with_capacity(sig.inputs.len());
        for (i, arg) in sig.inputs.iter_mut().enumerate() {
            if let FnArg::Typed(typed_arg) = arg {
                let arg = Ident::new(&format!("__arg{i}"), typed_arg.pat.span());
                typed_arg.pat = Box::new(syn::parse_quote!(#arg));

                if let Some(kind) = self.resource_args.get(&i) {
                    args.push(kind.prepare_for_import(&arg, cr));
                } else {
                    args.push(quote!(#arg));
                }
            }
        }

        let delegation = quote!(#new_ident(#(#args,)*));
        let delegation = match self.return_type {
            ReturnType::Resource(kind) => {
                let output = Ident::new("__output", sig.span());
                let init = kind.initialize_for_export(&output, cr);
                quote! {
                    let #output = #delegation;
                    #init
                }
            }
            ReturnType::NotResource => delegation,
            ReturnType::Default => quote!(#delegation;),
        };

        let wrapper = quote! {
            #[inline(never)]
            #vis #sig {
                unsafe { #cr::ExternRef::guard(); }
                #delegation
            }
        };
        (wrapper, new_ident)
    }

    fn create_externrefs(&self) -> impl ToTokens {
        let cr = &self.crate_path;
        let args_and_return_type_count = if matches!(self.return_type, ReturnType::Default) {
            self.arg_count
        } else {
            self.arg_count + 1
        };
        let bytes = (args_and_return_type_count + 7) / 8;

        let maybe_ret_idx = if matches!(self.return_type, ReturnType::Resource(_)) {
            Some(self.arg_count)
        } else {
            None
        };

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
            #cr::BitSlice::builder::<#bytes>(#args_and_return_type_count)
                #(#set_bits)*
                .build()
        }
    }
}

pub(crate) fn for_export(function: &mut ItemFn, attrs: &ExternrefAttrs) -> TokenStream {
    let parsed_function = match Function::new(function, attrs) {
        Ok(function) => function,
        Err(err) => return err.into_compile_error(),
    };
    let (declaration, export) = if parsed_function.needs_declaring() {
        // "Un-export" the function by removing the relevant attributes.
        function.sig.abi = None;
        let attr_idx = function.attrs.iter().enumerate().find_map(|(idx, attr)| {
            if attr.path().is_ident("export_name") {
                Some(idx)
            } else {
                None
            }
        });
        let export_name_attr = attr_idx.map(|idx| function.attrs.remove(idx));

        // Remove `#[no_mangle]` attr if present as well; if it is retained, it will still
        // generate an export.
        function
            .attrs
            .retain(|attr| !attr.path().is_ident("no_mangle"));

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

struct Imports {
    module_name: String,
    functions: Vec<(Function, TokenStream)>,
}

impl Imports {
    fn new(module: &mut ItemForeignMod, attrs: &ExternrefAttrs) -> Result<Self, SynError> {
        const NO_ATTR_MSG: &str = "#[link(wasm_import_module = \"..\")] must be specified \
            on the foreign module";

        check_abi("foreign module", module.abi.name.as_ref(), &module.abi)?;

        let link_attr = module
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("link"));
        let Some(link_attr) = link_attr else {
            return Err(SynError::new_spanned(module, NO_ATTR_MSG));
        };

        let module_name = if matches!(link_attr.meta, Meta::List(_)) {
            let nested =
                link_attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
            nested
                .into_iter()
                .find_map(|nested_meta| match nested_meta {
                    Meta::NameValue(nv) if nv.path.is_ident("wasm_import_module") => Some(nv.value),
                    _ => None,
                })
        } else {
            let msg =
                "Unexpected contents of `#[link(..)]` attr (expected a list of name-value pairs)";
            return Err(SynError::new_spanned(link_attr, msg));
        };

        let module_name =
            module_name.ok_or_else(|| SynError::new_spanned(link_attr, NO_ATTR_MSG))?;
        let module_name = if let Expr::Lit(ExprLit {
            lit: Lit::Str(str), ..
        }) = module_name
        {
            str.value()
        } else {
            let msg = "Unexpected WASM module name format (expected a string)";
            return Err(SynError::new(module_name.span(), msg));
        };

        let cr = attrs.crate_path();
        let mut functions = Vec::with_capacity(module.items.len());
        for item in &mut module.items {
            if let ForeignItem::Fn(fn_item) = item {
                let link_name = attr_expr(&fn_item.attrs, "link_name")?;
                let has_link_name = link_name.is_some();
                let function = Function::from_sig(&fn_item.sig, link_name, attrs);
                if !function.needs_declaring() {
                    continue;
                }

                let vis = mem::replace(&mut fn_item.vis, Visibility::Inherited);
                let (wrapper, new_ident) = function.wrap_import(&vis, fn_item.sig.clone());
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
                            typed_arg.ty = Box::new(syn::parse_quote!(#cr::ExternRef));
                        }
                    }
                }
                if matches!(function.return_type, ReturnType::Resource(_)) {
                    fn_item.sig.output = syn::parse_quote!(-> #cr::ExternRef);
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

pub(crate) fn for_foreign_module(
    module: &mut ItemForeignMod,
    attrs: &ExternrefAttrs,
) -> TokenStream {
    let parsed_module = match Imports::new(module, attrs) {
        Ok(module) => module,
        Err(err) => return err.into_compile_error(),
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
        let parsed = Function::new(&export_fn, &ExternrefAttrs::default()).unwrap();
        assert!(parsed.needs_declaring());

        let declaration = parsed.declare(None);
        let declaration: syn::Item = syn::parse_quote!(#declaration);
        let expected: syn::Item = syn::parse_quote! {
            externref::declare_function!(externref::Function {
                kind: externref::FunctionKind::Export,
                name: "test_export",
                externrefs: externref::BitSlice::builder::<1usize>(3usize)
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
                buffer: Option<Resource<Buffer>>,
                some_ptr: *const u8,
            ) {
                // does nothing
            }
        };
        let parsed = Function::new(&export_fn, &ExternrefAttrs::default()).unwrap();
        assert_eq!(parsed.resource_args.len(), 2);
        assert_eq!(parsed.resource_args[&0], SimpleResourceKind::MutRef.into());
        assert_eq!(
            parsed.resource_args[&1],
            ResourceKind::Option(SimpleResourceKind::Owned)
        );
        assert_eq!(parsed.return_type, ReturnType::Default);

        let wrapper = parsed.wrap_export(&export_fn, None);
        let wrapper: syn::Item = syn::parse_quote!(#wrapper);
        let expected: syn::Item = syn::parse_quote! {
            const _: () = {
                #[export_name = "test_export"]
                unsafe extern "C" fn __externref_export(
                    __arg0: externref::ExternRef,
                    __arg1: externref::ExternRef,
                    __arg2: *const u8,
                ) {
                    test_export(
                        externref::Resource::new(__arg0)
                            .as_mut()
                            .expect("null reference passed from host"),
                        externref::Resource::new(__arg1),
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
        let parsed = Function::from_sig(&sig, None, &ExternrefAttrs::default());

        let (wrapper, ident) = parsed.wrap_import(&Visibility::Inherited, sig);
        assert_eq!(ident, "__externref_send_message");

        let wrapper: ItemFn = syn::parse_quote!(#wrapper);
        let expected: ItemFn = syn::parse_quote! {
            #[inline(never)]
            unsafe fn send_message(
                __arg0: &Resource<Sender>,
                __arg1: *const u8,
                __arg2: usize,
            ) -> Resource<Bytes> {
                unsafe { externref::ExternRef::guard(); }
                let __output = __externref_send_message(
                    externref::Resource::raw(core::option::Option::Some(__arg0)),
                    __arg1,
                    __arg2,
                );
                externref::Resource::new(__output).expect("null reference passed from host")
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
        Imports::new(&mut foreign_mod, &ExternrefAttrs::default()).unwrap();

        let expected: ItemForeignMod = syn::parse_quote! {
            #[link(wasm_import_module = "test")]
            extern "C" {
                #[link_name = "send_message"]
                fn __externref_send_message(
                    sender: externref::ExternRef,
                    message_ptr: *const u8,
                    message_len: usize,
                ) -> externref::ExternRef;
            }
        };
        assert_eq!(foreign_mod, expected, "{}", quote!(#foreign_mod));
    }
}
