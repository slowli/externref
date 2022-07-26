use proc_macro2::TokenStream;
use quote::{quote, ToTokens};
use syn::{
    spanned::Spanned, Attribute, FnArg, ForeignItem, ItemFn, ItemForeignMod, Lit, LitStr, Meta,
    MetaList, NestedMeta, PatType, PathArguments, ReturnType, Signature, Type, TypePath,
};

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

#[derive(Debug)]
struct Function {
    name: String,
    arg_count: usize,
    resource_args: Vec<usize>,
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
                if Self::is_resource(ty) {
                    return Some(i);
                }
            }
            None
        });
        let resource_return_type = match &sig.output {
            ReturnType::Type(_, ty) => Some(Self::is_resource(ty)),
            ReturnType::Default => None,
        };

        Self {
            name: name_override.unwrap_or_else(|| sig.ident.to_string()),
            arg_count: sig.inputs.len(),
            resource_args: resource_args.collect(),
            resource_return_type,
        }
    }

    fn is_resource(ty: &Type) -> bool {
        if let Type::Path(TypePath { path, .. }) = ty {
            path.segments.last().map_or(false, |segment| {
                segment.ident == "Resource"
                    && matches!(
                        &segment.arguments,
                        PathArguments::AngleBracketed(args) if args.args.len() == 1
                    )
            })
        } else {
            false
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
        let set_bits = self.resource_args.iter().copied().chain(maybe_ret_idx);
        let set_bits = set_bits.map(|idx| quote!(.with_set_bit(#idx)));

        quote! {
            externref::signature::BitSlice::builder::<#bytes>(#args_and_return_type_count)
                #(#set_bits)*
                .build()
        }
    }
}

pub(crate) fn for_export(function: &ItemFn) -> TokenStream {
    let parsed_function = match Function::new(function) {
        Ok(function) => function,
        Err(err) => return err.write_errors(),
    };
    let declaration = if parsed_function.needs_declaring() {
        Some(parsed_function.declare(None))
    } else {
        None
    };

    quote! {
        #function
        #declaration
    }
}

#[derive(Debug)]
struct Imports {
    module_name: String,
    functions: Vec<Function>,
}

impl Imports {
    fn new(module: &ItemForeignMod) -> darling::Result<Self> {
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
        for item in &module.items {
            if let ForeignItem::Fn(function) = item {
                let link_name = attr_string(&function.attrs, "link_name")?;
                let function = Function::from_sig(&function.sig, link_name);
                if function.needs_declaring() {
                    functions.push(function);
                }
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
            .map(|function| function.declare(Some(&self.module_name)));
        quote!(#(#function_declarations)*)
    }
}

pub(crate) fn for_foreign_module(module: &ItemForeignMod) -> TokenStream {
    let parsed_module = match Imports::new(module) {
        Ok(module) => module,
        Err(err) => return err.write_errors(),
    };
    let declarations = parsed_module.declarations();
    quote! {
        #module
        #declarations
    }
}
