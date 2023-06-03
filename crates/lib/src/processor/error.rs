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
            Self::Arg(idx) => write!(formatter, "arg #{idx}"),
            Self::ReturnType(idx) => write!(formatter, "return type #{idx}"),
        }
    }
}

/// Errors that can occur when [processing] a WASM module.
///
/// [processing]: super::Processor::process()
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Error reading the custom section with function declarations from the module.
    Read(ReadError),
    /// Error parsing the WASM module.
    Wasm(anyhow::Error),

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

    /// Incorrectly placed `externref` guard. This is caused by processing the WASM module
    /// with external tools (e.g., `wasm-opt`) before using this processor.
    IncorrectGuard {
        /// Name of the function with an incorrectly placed guard.
        function_name: Option<String>,
        /// WASM bytecode offset of the offending guard.
        code_offset: Option<u32>,
    },
    /// Unexpected call to a function returning `externref`. Such calls should be confined
    /// in order for the processor to work properly. Like with [`Self::IncorrectGuard`],
    /// such errors should only be caused by external tools (e.g., `wasm-opt`).
    UnexpectedCall {
        /// Name of the function containing an unexpected call.
        function_name: Option<String>,
        /// WASM bytecode offset of the offending call.
        code_offset: Option<u32>,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        const EXTERNAL_TOOL_TIP: &str = "This can be caused by an external WASM manipulation tool \
            such as `wasm-opt`. Please run such tools *after* the externref processor.";

        match self {
            Self::Read(err) => write!(formatter, "failed reading WASM custom section: {err}"),
            Self::Wasm(err) => write!(formatter, "failed reading WASM module: {err}"),

            Self::UnexpectedImportType { module, name } => {
                write!(
                    formatter,
                    "unexpected type of import `{module}::{name}`; expected a function"
                )
            }

            Self::NoExport(name) => {
                write!(formatter, "missing exported function `{name}`")
            }
            Self::UnexpectedExportType(name) => {
                write!(
                    formatter,
                    "unexpected type of export `{name}`; expected a function"
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
                    .map_or_else(String::new, |module| format!(" imported from `{module}`"));
                write!(
                    formatter,
                    "unexpected arity for function `{name}`{module_descr}: \
                     expected {expected_arity}, got {real_arity}"
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
                    .map_or_else(String::new, |module| format!(" imported from `{module}`"));
                write!(
                    formatter,
                    "{location} of function `{name}`{module_descr} has unexpected type; \
                     expected `i32`, got {real_type}"
                )
            }

            Self::IncorrectGuard {
                function_name,
                code_offset,
            } => {
                let function_name = function_name
                    .as_ref()
                    .map_or("(unnamed function)", String::as_str);
                let code_offset = code_offset
                    .as_ref()
                    .map_or_else(String::new, |offset| format!(" at {offset}"));
                write!(
                    formatter,
                    "incorrectly placed externref guard in {function_name}{code_offset}. \
                     {EXTERNAL_TOOL_TIP}"
                )
            }
            Self::UnexpectedCall {
                function_name,
                code_offset,
            } => {
                let function_name = function_name
                    .as_ref()
                    .map_or("(unnamed function)", String::as_str);
                let code_offset = code_offset
                    .as_ref()
                    .map_or_else(String::new, |offset| format!(" at {offset}"));
                write!(
                    formatter,
                    "unexpected call to an `externref`-returning function \
                     in {function_name}{code_offset}. {EXTERNAL_TOOL_TIP}"
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
