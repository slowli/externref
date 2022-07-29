//! Processing errors.

use std::{error, fmt};

use crate::ReadError;

/// Location of a `Resource`: a function argument or a return type.
#[derive(Debug)]
pub enum Location {
    /// Argument with the specified zero-based index.
    Arg(usize),
    /// Return type with the specified zero-based index.
    ReturnType(usize),
}

impl fmt::Display for Location {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Arg(idx) => write!(formatter, "arg #{}", idx),
            Self::ReturnType(idx) => write!(formatter, "return type #{}", idx),
        }
    }
}

/// Errors that can occur when [processing] a WASM module.
///
/// [processing]: crate::Processor::process()
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Error reading the custom section with function declarations from the module.
    Read(ReadError),
    /// Error parsing the WASM module.
    Wasm(anyhow::Error),
    /// Missing imported function with the enclosed module / name.
    NoImport {
        /// Name of the module.
        module: String,
        /// Name of the function.
        name: String,
    },
    /// Unexpected type of an import (expected a function).
    UnexpectedImportType {
        /// Name of the module.
        module: String,
        /// Name of the function.
        name: String,
    },
    /// Missing exported function with the enclosed name.
    NoExport(String),
    /// Unexpected type of an export (expected a function).
    UnexpectedExportType(String),
    /// Imported or exported function has unexpected arity.
    UnexpectedArity {
        /// Name of the module; `None` for exported functions.
        module: Option<String>,
        /// Name of the function.
        name: String,
        /// Expected arity of the function.
        expected_arity: usize,
        /// Actual arity of the function.
        real_arity: usize,
    },
    /// Argument or return type of a function has unexpected type.
    UnexpectedType {
        /// Name of the module; `None` for exported functions.
        module: Option<String>,
        /// Name of the function.
        name: String,
        /// Location of an argument / return type in the function.
        location: Location,
        /// Actual type of the function (the expected type is always `i32`).
        real_type: walrus::ValType,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(err) => write!(formatter, "failed reading WASM custom section: {}", err),
            Self::Wasm(err) => write!(formatter, "failed reading WASM module: {}", err),

            Self::NoImport { module, name } => {
                write!(
                    formatter,
                    "missing imported function `{}::{}`",
                    module, name
                )
            }
            Self::UnexpectedImportType { module, name } => {
                write!(
                    formatter,
                    "unexpected type of import `{}::{}`; expected a function",
                    module, name
                )
            }

            Self::NoExport(name) => {
                write!(formatter, "missing exported function `{}`", name)
            }
            Self::UnexpectedExportType(name) => {
                write!(
                    formatter,
                    "unexpected type of export `{}`; expected a function",
                    name
                )
            }

            Self::UnexpectedArity {
                module,
                name,
                expected_arity,
                real_arity,
            } => {
                let module_descr = module
                    .as_ref()
                    .map_or_else(String::new, |module| format!(" imported from `{}`", module));
                write!(
                    formatter,
                    "unexpected arity for function `{}`{}: expected {}, got {}",
                    name, module_descr, expected_arity, real_arity
                )
            }
            Self::UnexpectedType {
                module,
                name,
                location,
                real_type,
            } => {
                let module_descr = module
                    .as_ref()
                    .map_or_else(String::new, |module| format!(" imported from `{}`", module));
                write!(
                    formatter,
                    "{} of function `{}`{} has unexpected type; expected `i32`, got {}",
                    location, name, module_descr, real_type
                )
            }
        }
    }
}

impl From<ReadError> for Error {
    fn from(err: ReadError) -> Self {
        Self::Read(err)
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Read(err) => Some(err),
            Self::Wasm(err) => Some(err.as_ref()),
            _ => None,
        }
    }
}
