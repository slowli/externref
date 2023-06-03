//! WASM module processor for `externref`s.
//!
//! WASM modules that use the `externref` crate need to be processed in order
//! to use `externref` function args / return types for imported or exported functions that
//! originally used [`Resource`](crate::Resource)s. This module encapsulates processing logic.
//!
//! More precisely, the processor performs the following steps:
//!
//! - Parse the custom section with [`Function`] declarations and remove this section
//!   from the module.
//! - Replace imported functions from a surrogate module for handling `externref`s with
//!   local functions.
//! - Patch signatures and implementations of imported / exported functions so that they
//!   use `externref`s where appropriate.
//! - Add an initially empty, unconstrained table with `externref` elements and optionally
//!   export it from the module. The host can use the table to inspect currently used references
//!   (e.g., to save / restore WASM instance state).
//!
//! See [crate-level docs](..) for more insights on WASM module setup and processing.
//!
//! # On processing order
//!
//! ⚠ **Important.** The [`Processor`] should run *before* WASM optimization tools such as `wasm-opt`.
//! These tools may inline `externref`-operating functions, which can lead to the processor
//! producing invalid WASM bytecode (roughly speaking, excessively replacing `i32`s
//! with `externref`s). Such inlining can usually be detected by the processor, in which case
//! it will return [`Error::IncorrectGuard`] or [`Error::UnexpectedCall`]
//! from [`process()`](Processor::process()).
//!
//! Optimizing WASM after the processor has an additional advantage in that it can
//! optimize the changes produced by it (optimization is hard, and is best left
//! to the dedicated tools).
//!
//! # Examples
//!
//! ```
//! use externref::processor::Processor;
//!
//! let module: Vec<u8> = // WASM module, e.g., loaded from the file system
//! #    b"\0asm\x01\0\0\0".to_vec();
//! let processed: Vec<u8> = Processor::default()
//!     // Set a hook to be called when a reference is dropped
//!     .set_drop_fn("test", "drop_ref")
//!     .process_bytes(&module)?;
//! // Store or use the processed module...
//! # Ok::<_, externref::processor::Error>(())
//! ```

use walrus::{passes::gc, Module};

mod error;
mod functions;
mod state;

pub use self::error::{Error, Location};
use self::state::ProcessingState;
use crate::Function;

/// WASM module processor encapsulating processing options.
#[derive(Debug)]
pub struct Processor<'a> {
    table_name: Option<&'a str>,
    drop_fn_name: Option<(&'a str, &'a str)>,
}

impl Default for Processor<'_> {
    fn default() -> Self {
        Self {
            table_name: Some("externrefs"),
            drop_fn_name: None,
        }
    }
}

impl<'a> Processor<'a> {
    /// Sets the name of the exported `externref`s table where refs obtained from the host
    /// are placed. If set to `None`, the table will not be exported from the module.
    ///
    /// By default, the table is exported as `"externrefs"`.
    pub fn set_ref_table(&mut self, name: impl Into<Option<&'a str>>) -> &mut Self {
        self.table_name = name.into();
        self
    }

    /// Sets a function to notify the host about dropped `externref`s. This function
    /// will be added as an import with a signature `(externref) -> ()` and will be called
    /// immediately before dropping each reference.
    ///
    /// By default, there is no notifier hook installed.
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
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all, err))]
    pub fn process(&self, module: &mut Module) -> Result<(), Error> {
        let raw_section = module.customs.remove_raw(Function::CUSTOM_SECTION_NAME);
        let Some(raw_section) = raw_section else {
            #[cfg(feature = "tracing")]
            tracing::info!("module contains no custom section; skipping");
            return Ok(());
        };
        let functions = Self::parse_section(&raw_section.data)?;
        #[cfg(feature = "tracing")]
        tracing::info!(functions.len = functions.len(), "parsed custom section");

        let state = ProcessingState::new(module, self)?;
        let guarded_fns = state.replace_functions(module)?;
        state.process_functions(&functions, &guarded_fns, module)?;

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
