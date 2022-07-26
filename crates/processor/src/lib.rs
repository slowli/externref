//! WASM post-processor for `externref`.

use walrus::{passes::gc, Module};

use externref::signature::Function;

mod error;
mod functions;
mod state;

pub use crate::error::{Error, Location};
use crate::state::ProcessingState;

/// Processes the provided `module`.
pub fn process(module: &mut Module) -> Result<(), Error> {
    let raw_section = if let Some(section) = module.customs.remove_raw("__externrefs") {
        section
    } else {
        return Ok(());
    };
    let functions = parse_section(&raw_section.data)?;
    let state = ProcessingState::new(module)?;
    state.replace_functions(module);
    for function in &functions {
        state.process_function(function, module)?;
    }

    gc::run(module);
    Ok(())
}

pub fn process_bytes(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let mut module = Module::from_buffer(bytes).map_err(Error::Wasm)?;
    process(&mut module)?;
    Ok(module.emit_wasm())
}

fn parse_section(mut raw_section: &[u8]) -> Result<Vec<Function<'_>>, Error> {
    let mut functions = vec![];
    while !raw_section.is_empty() {
        let next_function = Function::read_from_section(&mut raw_section)?;
        functions.push(next_function);
    }
    Ok(functions)
}
