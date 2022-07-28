//! WASM post-processor for `externref`.

// Linter settings.
#![warn(missing_debug_implementations, missing_docs, bare_trait_objects)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::must_use_candidate, clippy::module_name_repetitions)]

use walrus::{passes::gc, Module};

use externref::signature::Function;

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
    /// Returns an error if a module is malformed. This shouldn't happen and could be caused
    /// by another post-processor or a bug in the `externref` crate / proc macro.
    pub fn process(&self, module: &mut Module) -> Result<(), Error> {
        let raw_section = if let Some(section) = module.customs.remove_raw("__externrefs") {
            section
        } else {
            return Ok(());
        };
        let functions = Self::parse_section(&raw_section.data)?;
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
