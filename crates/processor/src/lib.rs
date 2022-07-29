//! WASM post-processor for [`externref`](::externref).
//!
//! The processor must be used on WASM modules that use the `externref` crate in order
//! to produce a module with `externref` function args / return types for functions that
//! originally used [`Resource`]s. More precisely, the processor performs the following steps:
//!
//! - Parse the custom section with [`Function`] declarations and remove this section.
//! - Replace imported functions from a surrogate module for handling `externref`s with
//!   local functions.
//! - Patch signatures and implementations of imported / exported functions so that they
//!   use `externref`s where appropriate.
//!
//! See `externref` crate docs for more details on processing.
//!
//! # Crate features
//!
//! ## `log`
//!
//! *(Off by default)*
//!
//! Enables logging key events during processing with the [`log`] facade. Logs use
//! the `externref` target and mostly `INFO` and `DEBUG` levels.
//!
//! [`Resource`]: externref::Resource
//! [`Function`]: externref::Function
//! [`log`]: https://docs.rs/log/
//!
//! # Examples
//!
//! ```
//! use externref_processor::Processor;
//!
//! let module: Vec<u8> = // WASM module, e.g., loaded from the file system
//! #    b"\0asm\x01\0\0\0".to_vec();
//! let processed: Vec<u8> = Processor::default()
//!     // Set a hook to be called when a reference is dropped
//!     .set_drop_fn("test", "drop_ref")
//!     .process_bytes(&module)?;
//! // Store or use the processed module...
//! # Ok::<_, externref_processor::Error>(())
//! ```

// Linter settings.
#![warn(missing_debug_implementations, missing_docs, bare_trait_objects)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::must_use_candidate, clippy::module_name_repetitions)]

use walrus::{passes::gc, Module};

use externref::Function;

mod error;
mod functions;
mod state;

pub use crate::error::{Error, Location};
use crate::state::ProcessingState;

/// WASM module processor encapsulating processing options.
#[derive(Debug)]
pub struct Processor<'a> {
    table_name: &'a str,
    drop_fn_name: Option<(&'a str, &'a str)>,
}

impl Default for Processor<'_> {
    fn default() -> Self {
        Self {
            table_name: "externrefs",
            drop_fn_name: None,
        }
    }
}

impl<'a> Processor<'a> {
    /// Sets a function to notify the host about dropped `externref`s. This function
    /// will be added as an import with a signature `(externref) -> ()` and will be called
    /// immediately before dropping each reference.
    pub fn set_drop_fn(&mut self, module: &'a str, name: &'a str) -> &mut Self {
        self.drop_fn_name = Some((module, name));
        self
    }

    /// Processes the provided `module`.
    ///
    /// # Errors
    ///
    /// Returns an error if a module is malformed. This shouldn't normally happen and
    /// could be caused by another post-processor or a bug in the `externref` crate / proc macro.
    pub fn process(&self, module: &mut Module) -> Result<(), Error> {
        let raw_section = module.customs.remove_raw(Function::CUSTOM_SECTION_NAME);
        let raw_section = if let Some(section) = raw_section {
            section
        } else {
            #[cfg(feature = "log")]
            log::info!(target: "externref", "Module contains no custom section; skipping");
            return Ok(());
        };
        let functions = Self::parse_section(&raw_section.data)?;
        #[cfg(feature = "log")]
        Self::log_functions(&functions);

        let state = ProcessingState::new(module, self)?;
        state.replace_functions(module);
        for function in &functions {
            ProcessingState::process_function(function, module)?;
        }

        gc::run(module);
        Ok(())
    }

    fn parse_section(mut raw_section: &[u8]) -> Result<Vec<Function<'_>>, Error> {
        let mut functions = vec![];
        while !raw_section.is_empty() {
            let next_function = Function::read_from_section(&mut raw_section)?;
            functions.push(next_function);
        }
        Ok(functions)
    }

    #[cfg(feature = "log")]
    fn log_functions(functions: &[Function<'_>]) {
        use externref::FunctionKind;

        log::info!(target: "externref", "Custom section contains {} functions", functions.len());
        for function in functions {
            let origin = match &function.kind {
                FunctionKind::Export => "exported".to_owned(),
                FunctionKind::Import(module) => format!("imported from module {}", module),
            };
            let ref_count = function.externrefs.count_ones();
            log::info!(
                target: "externref",
                "- `{}`: {}, with {} externref(s)",
                function.name, origin, ref_count
            );
        }
    }

    /// Processes the provided WASM module `bytes`. This is a higher-level alternative to
    /// [`Self::process()`].
    ///
    /// # Errors
    ///
    /// Returns an error if `bytes` does not represent a valid WASM module, and in all cases
    /// [`Self::process()`] returns an error.
    pub fn process_bytes(&self, bytes: &[u8]) -> Result<Vec<u8>, Error> {
        let mut module = Module::from_buffer(bytes).map_err(Error::Wasm)?;
        self.process(&mut module)?;
        Ok(module.emit_wasm())
    }
}
