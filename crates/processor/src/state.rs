//! Stateful WASM module processing.

use walrus::{
    ir, ExportItem, FunctionBuilder, FunctionId, ImportKind, LocalId, Module, ModuleTypes, TypeId,
    ValType,
};

use std::{collections::HashMap, mem};

use crate::{
    functions::{ExternrefImports, PatchedFunctions},
    Error, Location,
};
use externref::signature::{Function, FunctionKind};

#[derive(Debug)]
pub(crate) struct ProcessingState {
    patched_fns: PatchedFunctions,
}

impl ProcessingState {
    pub fn new(module: &mut Module) -> Result<Self, Error> {
        let imports = ExternrefImports::new(&mut module.imports)?;
        let patched_fns = PatchedFunctions::new(module, &imports);
        Ok(Self { patched_fns })
    }

    pub fn replace_functions(&self, module: &mut Module) {
        self.patched_fns.replace_calls(module);
    }

    pub fn process_function(function: &Function<'_>, module: &mut Module) -> Result<(), Error> {
        match function.kind {
            FunctionKind::Export => {
                let export = module
                    .exports
                    .iter()
                    .find(|export| export.name == function.name);
                let export = export.ok_or_else(|| Error::NoExport(function.name.to_owned()))?;
                let fn_id = match &export.item {
                    ExportItem::Function(fn_id) => *fn_id,
                    _ => return Err(Error::UnexpectedExportType(function.name.to_owned())),
                };
                Self::transform_local_fn(module, fn_id, function)?;
            }

            FunctionKind::Import(module_name) => {
                let import_id =
                    module
                        .imports
                        .find(module_name, function.name)
                        .ok_or_else(|| Error::NoImport {
                            module: module_name.to_owned(),
                            name: function.name.to_owned(),
                        })?;
                let fn_id = match module.imports.get(import_id).kind {
                    ImportKind::Function(fn_id) => fn_id,
                    _ => {
                        return Err(Error::UnexpectedImportType {
                            module: module_name.to_owned(),
                            name: function.name.to_owned(),
                        })
                    }
                };
                transform_imported_fn(module, function, fn_id)?;
            }
        }

        Ok(())
    }

    fn transform_local_fn(
        module: &mut Module,
        fn_id: FunctionId,
        function: &Function<'_>,
    ) -> Result<(), Error> {
        let local_fn = module.funcs.get_mut(fn_id).kind.unwrap_local_mut();
        let (params, results) = patch_type_inner(&module.types, function, local_fn.ty())?;

        let mut locals_mapping = HashMap::new();
        for idx in function.externrefs.set_indices() {
            if let Some(arg) = local_fn.args.get_mut(idx) {
                let new_local = module.locals.add(ValType::Externref);
                locals_mapping.insert(*arg, new_local);
                *arg = new_local;
            }
        }

        // Determine which `local.get $arg` instructions must be replaced with new arg locals.
        let mut locals_visitor = ReplaceLocals::new(locals_mapping.keys().copied());
        ir::dfs_in_order(&mut locals_visitor, local_fn, local_fn.entry_block());
        // Clone the function with new function types.
        let mut visitor =
            CloneFunction::new(FunctionBuilder::new(&mut module.types, &params, &results));
        ir::dfs_in_order(&mut visitor, local_fn, local_fn.entry_block());

        // We cannot use `VisitorMut` here because we're switching arenas for `InstrSeqId`s.
        for (old_id, new_id) in &visitor.sequence_mapping {
            let seq = local_fn.block_mut(*old_id);
            let mut instructions = mem::take(&mut seq.instrs);
            for (instr, _) in &mut instructions {
                match instr {
                    ir::Instr::Block(ir::Block { seq })
                    | ir::Instr::Loop(ir::Loop { seq })
                    | ir::Instr::Br(ir::Br { block: seq })
                    | ir::Instr::BrIf(ir::BrIf { block: seq }) => {
                        *seq = visitor.sequence_mapping[seq];
                    }

                    ir::Instr::IfElse(ir::IfElse {
                        consequent,
                        alternative,
                    }) => {
                        *consequent = visitor.sequence_mapping[consequent];
                        *alternative = visitor.sequence_mapping[alternative];
                    }
                    ir::Instr::BrTable(ir::BrTable { blocks, default }) => {
                        for block in blocks.iter_mut() {
                            *block = visitor.sequence_mapping[block];
                        }
                        *default = visitor.sequence_mapping[default];
                    }

                    ir::Instr::LocalGet(ir::LocalGet { local }) => {
                        if locals_visitor.should_replace(*old_id, *local) {
                            *local = locals_mapping[local];
                        }
                    }

                    _ => { /* Do nothing */ }
                }
            }

            *visitor.builder.instr_seq(*new_id).instrs_mut() = instructions;
        }

        *local_fn.builder_mut() = visitor.builder;
        Ok(())
    }
}

#[derive(Debug, Default)]
struct LocalState {
    seq_counts: HashMap<ir::InstrSeqId, usize>,
    reassigned: bool,
}

/// Visitor replacing mentions of `externref` args of patched functions.
///
/// It is valid to reassign param locals via `local.set` or `local.tee`
/// (and Rust frequently does this in practice).
/// Since we change the local type from `i32` to `externref`, we need to track reassignments,
/// and not change the local ID after reassignment (since it should retain the old `i32` type).
#[derive(Debug)]
struct ReplaceLocals {
    locals: HashMap<LocalId, LocalState>,
    current_seqs: Vec<ir::InstrSeqId>,
}

impl ReplaceLocals {
    fn new(locals: impl Iterator<Item = LocalId>) -> Self {
        Self {
            locals: locals
                .map(|local_id| (local_id, LocalState::default()))
                .collect(),
            current_seqs: vec![],
        }
    }

    fn should_replace(&mut self, seq: ir::InstrSeqId, local: LocalId) -> bool {
        if let Some(state) = self.locals.get_mut(&local) {
            if let Some(count) = state.seq_counts.get_mut(&seq) {
                if *count > 0 {
                    *count -= 1;
                    return true;
                }
            }
        }
        false
    }
}

impl ir::Visitor<'_> for ReplaceLocals {
    fn start_instr_seq(&mut self, instr_seq: &ir::InstrSeq) {
        self.current_seqs.push(instr_seq.id());
    }

    fn end_instr_seq(&mut self, _: &ir::InstrSeq) {
        self.current_seqs.pop();
    }

    fn visit_local_id(&mut self, local_id: &LocalId) {
        let current_seq = *self.current_seqs.last().unwrap();
        if let Some(state) = self.locals.get_mut(local_id) {
            if !state.reassigned {
                *state.seq_counts.entry(current_seq).or_insert(0) += 1;
            }
        }
    }

    fn visit_local_set(&mut self, instr: &ir::LocalSet) {
        if let Some(state) = self.locals.get_mut(&instr.local) {
            state.reassigned = true;
        }
    }

    fn visit_local_tee(&mut self, instr: &ir::LocalTee) {
        if let Some(state) = self.locals.get_mut(&instr.local) {
            state.reassigned = true;
        }
    }
}

/// Visitor for function cloning.
#[derive(Debug)]
struct CloneFunction {
    builder: FunctionBuilder,
    sequence_mapping: HashMap<ir::InstrSeqId, ir::InstrSeqId>,
}

impl CloneFunction {
    fn new(builder: FunctionBuilder) -> Self {
        Self {
            builder,
            sequence_mapping: HashMap::new(),
        }
    }
}

impl ir::Visitor<'_> for CloneFunction {
    fn start_instr_seq(&mut self, instr_seq: &ir::InstrSeq) {
        let new_id = if self.sequence_mapping.is_empty() {
            // entry block
            self.builder.func_body().id()
        } else {
            self.builder.dangling_instr_seq(instr_seq.ty).id()
        };
        self.sequence_mapping.insert(instr_seq.id(), new_id);
    }
}

fn transform_imported_fn(
    module: &mut Module,
    function: &Function<'_>,
    fn_id: FunctionId,
) -> Result<(), Error> {
    let imported_fn = module.funcs.get_mut(fn_id).kind.unwrap_import_mut();
    let patched_ty = patch_type(&mut module.types, function, imported_fn.ty)?;
    imported_fn.ty = patched_ty;
    Ok(())
}

fn patch_type(
    types: &mut ModuleTypes,
    function: &Function<'_>,
    ty: TypeId,
) -> Result<TypeId, Error> {
    let (params, results) = patch_type_inner(types, function, ty)?;
    Ok(types.add(&params, &results))
}

fn patch_type_inner(
    types: &ModuleTypes,
    function: &Function<'_>,
    ty: TypeId,
) -> Result<(Vec<ValType>, Vec<ValType>), Error> {
    let (params, results) = types.params_results(ty);
    if params.len() + results.len() != function.externrefs.bit_len() {
        return Err(Error::UnexpectedArity {
            module: function.kind.module().map(str::to_owned),
            name: function.name.to_owned(),
            expected_arity: function.externrefs.bit_len(),
            real_arity: params.len() + results.len(),
        });
    }

    let mut params = params.to_vec();
    let mut results = results.to_vec();
    for idx in function.externrefs.set_indices() {
        let placement = if idx < params.len() {
            &mut params[idx]
        } else {
            &mut results[idx - params.len()]
        };

        if *placement != ValType::I32 {
            return Err(Error::UnexpectedType {
                module: function.kind.module().map(str::to_owned),
                name: function.name.to_owned(),
                location: if idx < params.len() {
                    Location::Arg(idx)
                } else {
                    Location::ReturnType(idx - params.len())
                },
                real_type: params[idx],
            });
        }
        *placement = ValType::Externref;
    }
    Ok((params, results))
}
