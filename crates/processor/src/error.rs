//! Processing errors.

use std::{error, fmt};

use externref::signature::ReadError;

#[derive(Debug)]
pub enum Location {
    Arg(usize),
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

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Read(ReadError),
    Wasm(anyhow::Error),

    /// Missing imported function with the enclosed module / name.
    NoImport {
        module: String,
        name: String,
    },
    /// Unexpected type of an import (expected a function).
    UnexpectedImportType {
        module: String,
        name: String,
    },
    /// Missing exported function with the enclosed name.
    NoExport(String),
    /// Unexpected type of an export (expected a function).
    UnexpectedExportType(String),
    /// Imported or exported function has unexpected arity.
    UnexpectedArity {
        module: Option<String>,
        name: String,
        expected_arity: usize,
        real_arity: usize,
    },
    /// Argument or return type of a function has unexpected type.
    UnexpectedType {
        module: Option<String>,
        name: String,
        location: Location,
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
