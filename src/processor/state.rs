//! Stateful WASM module processing.

use walrus::{
    ir, ExportItem, FunctionBuilder, FunctionId, ImportKind, LocalFunction, LocalId, Module,
    ModuleLocals, ModuleTypes, TypeId, ValType,
};

use std::{
    collections::{HashMap, HashSet},
    iter, mem,
};

use super::{
    functions::{ExternrefImports, PatchedFunctions},
    Error, Location, Processor,
};
use crate::{Function, FunctionKind};

#[derive(Debug)]
pub(crate) struct ProcessingState {
    patched_fns: PatchedFunctions,
}

impl ProcessingState {
    pub fn new(module: &mut Module, processor: &Processor<'_>) -> Result<Self, Error> {
        let imports = ExternrefImports::new(&mut module.imports)?;
        let patched_fns = PatchedFunctions::new(module, &imports, processor);
        Ok(Self { patched_fns })
    }

    #[cfg(feature = "processor-log")]
    pub fn replace_functions(&self, module: &mut Module) {
        let replaced_count = self.patched_fns.replace_calls(module);
        log::info!(
            target: "externref",
            "Replaced {} calls to externref imports",
            replaced_count
        );
    }

    #[cfg(not(feature = "processor-log"))]
    pub fn replace_functions(&self, module: &mut Module) {
        self.patched_fns.replace_calls(module);
    }

    pub fn process_functions(
        &self,
        functions: &[Function<'_>],
        module: &mut Module,
    ) -> Result<(), Error> {
        // First, resolve function IDs for exports / imports.
        let function_ids: Result<Vec<_>, _> = functions
            .iter()
            .map(|function| Self::function_id(function, module))
            .collect();
        let function_ids = function_ids?;

        // Determine which functions return externrefs (only patched imports or exports can
        // do that).
        let mut functions_returning_ref = HashSet::new();
        if let Some(fn_id) = self.patched_fns.get_ref_id() {
            functions_returning_ref.insert(fn_id);
        }

        for (function, &fn_id) in functions.iter().zip(&function_ids) {
            if let Some(fn_id) = fn_id {
                let type_id = module.funcs.get(fn_id).ty();
                let results_len = module.types.get(type_id).results().len();
                let refs = &function.externrefs;
                if results_len == 1 && refs.is_set(refs.bit_len() - 1) {
                    functions_returning_ref.insert(fn_id);
                }

                #[cfg_attr(not(feature = "processor-log"), allow(unused_variables))]
                if let FunctionKind::Import(module_name) = function.kind {
                    #[cfg(feature = "processor-log")]
                    log::info!(
                        target: "externref",
                        "Patching imported function `{}` from module `{}`",
                        function.name, module_name
                    );
                    transform_imported_fn(module, function, fn_id)?;
                }
            }
        }

        let functions_by_id = function_ids
            .into_iter()
            .zip(functions)
            .filter_map(|(fn_id, function)| fn_id.map(|fn_id| (fn_id, function)));
        let functions_by_id: HashMap<_, _> = functions_by_id.collect();

        let local_fn_ids: Vec<_> = module.funcs.iter_local().map(|(id, _)| id).collect();
        for fn_id in local_fn_ids {
            if let Some(function) = functions_by_id.get(&fn_id) {
                Self::transform_export(module, &functions_returning_ref, fn_id, function)?;
            } else {
                Self::transform_local_fn(module, &functions_returning_ref, fn_id);
            }
        }

        Ok(())
    }

    fn function_id(function: &Function<'_>, module: &Module) -> Result<Option<FunctionId>, Error> {
        Ok(Some(match function.kind {
            FunctionKind::Export => {
                let export = module
                    .exports
                    .iter()
                    .find(|export| export.name == function.name);
                let export = export.ok_or_else(|| Error::NoExport(function.name.to_owned()))?;
                match &export.item {
                    ExportItem::Function(fn_id) => *fn_id,
                    _ => return Err(Error::UnexpectedExportType(function.name.to_owned())),
                }
            }

            FunctionKind::Import(module_name) => {
                let import_id = match module.imports.find(module_name, function.name) {
                    Some(id) => id,
                    None => {
                        // The function is declared, but not actually used from the module.
                        // This is fine for us.
                        return Ok(None);
                    }
                };
                match module.imports.get(import_id).kind {
                    ImportKind::Function(fn_id) => fn_id,
                    _ => {
                        return Err(Error::UnexpectedImportType {
                            module: module_name.to_owned(),
                            name: function.name.to_owned(),
                        })
                    }
                }
            }
        }))
    }

    #[allow(clippy::needless_collect)] // false positive
    fn transform_export(
        module: &mut Module,
        functions_returning_ref: &HashSet<FunctionId>,
        fn_id: FunctionId,
        function: &Function<'_>,
    ) -> Result<(), Error> {
        #[cfg(feature = "processor-log")]
        log::info!(target: "externref", "Patching exported function `{}`", function.name);

        let local_fn = module.funcs.get_mut(fn_id).kind.unwrap_local_mut();
        let (params, results) = patch_type_inner(&module.types, function, local_fn.ty())?;

        let mut locals_mapping = HashMap::new();
        for idx in function.externrefs.set_indices() {
            if let Some(arg) = local_fn.args.get_mut(idx) {
                let new_local = module.locals.add(ValType::Externref);
                locals_mapping.insert(new_local, *arg);
                *arg = new_local;
            }
        }
        let ref_args: Vec<_> = locals_mapping.keys().copied().collect();

        let mut calls_visitor = RefCallDetector {
            locals: &mut module.locals,
            functions_returning_ref,
            new_locals: HashMap::default(),
        };
        ir::dfs_pre_order_mut(&mut calls_visitor, local_fn, local_fn.entry_block());
        let mut new_locals = calls_visitor.new_locals;
        new_locals.extend(locals_mapping);

        // Determine which `local.get $arg` instructions must be replaced with new arg locals.
        let mut locals_visitor = LocalReplacementCounter::new(ref_args.into_iter(), new_locals);
        ir::dfs_in_order(&mut locals_visitor, local_fn, local_fn.entry_block());
        let mut replacer = LocalReplacer::from(locals_visitor);
        // Clone the function with new function types.
        let mut cloner =
            FunctionCloner::new(FunctionBuilder::new(&mut module.types, &params, &results));
        ir::dfs_in_order(&mut cloner, local_fn, local_fn.entry_block());
        cloner.clone_function(local_fn, &mut replacer);

        Ok(())
    }

    fn transform_local_fn(
        module: &mut Module,
        functions_returning_ref: &HashSet<FunctionId>,
        fn_id: FunctionId,
    ) {
        let local_fn = module.funcs.get_mut(fn_id).kind.unwrap_local_mut();

        let mut calls_visitor = RefCallDetector {
            locals: &mut module.locals,
            functions_returning_ref,
            new_locals: HashMap::default(),
        };
        ir::dfs_pre_order_mut(&mut calls_visitor, local_fn, local_fn.entry_block());
        let new_locals = calls_visitor.new_locals;
        if new_locals.is_empty() {
            // No new locals are introduced by calls; the function doesn't need
            // to be transformed.
            return;
        }

        // Determine which `local.get $arg` instructions must be replaced with new arg locals.
        let mut locals_visitor = LocalReplacementCounter::new(iter::empty(), new_locals);
        ir::dfs_in_order(&mut locals_visitor, local_fn, local_fn.entry_block());
        let mut replacer = LocalReplacer::from(locals_visitor);
        ir::dfs_pre_order_mut(&mut replacer, local_fn, local_fn.entry_block());
    }
}

/// Visitor to detect calls to functions returning `externref`s and create a new ref local
/// for each call.
#[derive(Debug)]
struct RefCallDetector<'a> {
    locals: &'a mut ModuleLocals,
    functions_returning_ref: &'a HashSet<FunctionId>,
    /// Mapping from a new local to the old local.
    new_locals: HashMap<LocalId, LocalId>,
}

impl RefCallDetector<'_> {
    fn returns_ref(&self, instr: &ir::Instr) -> bool {
        if let ir::Instr::Call(call) = instr {
            self.functions_returning_ref.contains(&call.func)
        } else {
            false
        }
    }

    fn replace_local(&mut self, local: &mut LocalId) {
        let new_local = self.locals.add(ValType::Externref);
        self.new_locals.insert(new_local, *local);
        *local = new_local;
    }
}

impl ir::VisitorMut for RefCallDetector<'_> {
    fn start_instr_seq_mut(&mut self, instr_seq: &mut ir::InstrSeq) {
        let mut ref_on_top_of_stack = false;
        for (instr, _) in &mut instr_seq.instrs {
            match instr {
                ir::Instr::LocalSet(local_set) if ref_on_top_of_stack => {
                    self.replace_local(&mut local_set.local);
                    ref_on_top_of_stack = false;
                }
                ir::Instr::LocalTee(local_tee) if ref_on_top_of_stack => {
                    self.replace_local(&mut local_tee.local);
                }
                _ => {
                    ref_on_top_of_stack = self.returns_ref(instr);
                }
            }
        }
    }
}

#[derive(Debug, Default)]
struct LocalState {
    replacements: HashMap<ir::InstrSeqId, Vec<Option<LocalId>>>,
    current_replacement: Option<LocalId>,
}

/// Visitor counting mentions of `externref` locals in patched functions.
///
/// It is valid to reassign param locals via `local.set` or `local.tee`
/// (and Rust frequently does this in practice).
/// Since we change the local type from `i32` to `externref`, we need to track reassignments,
/// and not change the local ID after reassignment (since it should retain the old `i32` type).
#[derive(Debug)]
struct LocalReplacementCounter {
    locals: HashMap<LocalId, LocalState>,
    new_locals: HashMap<LocalId, LocalId>,
    current_seqs: Vec<ir::InstrSeqId>,
}

impl LocalReplacementCounter {
    fn new(ref_args: impl Iterator<Item = LocalId>, new_locals: HashMap<LocalId, LocalId>) -> Self {
        let mut locals: HashMap<_, _> = new_locals
            .values()
            .map(|local_id| (*local_id, LocalState::default()))
            .collect();
        for arg in ref_args {
            let old_local = new_locals[&arg];
            locals.get_mut(&old_local).unwrap().current_replacement = Some(arg);
        }

        Self {
            locals,
            new_locals,
            current_seqs: vec![],
        }
    }

    fn visit_assignment(&mut self, local: LocalId) {
        if let Some(state) = self.locals.get_mut(&local) {
            state.current_replacement = None;
        } else if let Some(old_local) = self.new_locals.get(&local) {
            let state = self.locals.get_mut(old_local).unwrap();
            state.current_replacement = Some(local);
        }
    }
}

impl ir::Visitor<'_> for LocalReplacementCounter {
    fn start_instr_seq(&mut self, instr_seq: &ir::InstrSeq) {
        self.current_seqs.push(instr_seq.id());
    }

    fn end_instr_seq(&mut self, _: &ir::InstrSeq) {
        self.current_seqs.pop();
    }

    fn visit_local_get(&mut self, instr: &ir::LocalGet) {
        let local_id = instr.local;
        let current_seq = *self.current_seqs.last().unwrap();
        if let Some(state) = self.locals.get_mut(&local_id) {
            state
                .replacements
                .entry(current_seq)
                .or_default()
                .push(state.current_replacement);
        }
    }

    fn visit_local_set(&mut self, instr: &ir::LocalSet) {
        self.visit_assignment(instr.local);
    }

    fn visit_local_tee(&mut self, instr: &ir::LocalTee) {
        self.visit_assignment(instr.local);
    }
}

#[derive(Debug)]
struct LocalReplacer {
    locals: HashMap<LocalId, LocalState>,
    current_seqs: Vec<ir::InstrSeqId>,
}

impl LocalReplacer {
    fn take_replacement(&mut self, seq: ir::InstrSeqId, local: LocalId) -> Option<LocalId> {
        if let Some(state) = self.locals.get_mut(&local) {
            if let Some(replacements) = state.replacements.get_mut(&seq) {
                return replacements.pop().flatten();
            }
        }
        None
    }
}

impl From<LocalReplacementCounter> for LocalReplacer {
    fn from(counter: LocalReplacementCounter) -> Self {
        // Reverse all replacements to pop them in `Self::take_replacement()` in proper order.
        let mut locals = counter.locals;
        for state in locals.values_mut() {
            for replacements in state.replacements.values_mut() {
                replacements.reverse();
            }
        }

        Self {
            locals,
            current_seqs: vec![],
        }
    }
}

impl ir::VisitorMut for LocalReplacer {
    fn start_instr_seq_mut(&mut self, instr_seq: &mut ir::InstrSeq) {
        self.current_seqs.push(instr_seq.id());
    }

    fn end_instr_seq_mut(&mut self, _: &mut ir::InstrSeq) {
        self.current_seqs.pop();
    }

    fn visit_local_get_mut(&mut self, instr: &mut ir::LocalGet) {
        let seq = self.current_seqs.last().unwrap();
        if let Some(replacement) = self.take_replacement(*seq, instr.local) {
            instr.local = replacement;
        }
    }
}

/// Visitor for function cloning.
#[derive(Debug)]
struct FunctionCloner {
    builder: FunctionBuilder,
    sequence_mapping: HashMap<ir::InstrSeqId, ir::InstrSeqId>,
}

impl FunctionCloner {
    fn new(builder: FunctionBuilder) -> Self {
        Self {
            builder,
            sequence_mapping: HashMap::new(),
        }
    }

    fn clone_function(self, local_fn: &mut LocalFunction, replacer: &mut LocalReplacer) {
        let mut builder = self.builder;
        // We cannot use `VisitorMut` here because we're switching arenas for `InstrSeqId`s.
        for (old_id, new_id) in &self.sequence_mapping {
            let seq = local_fn.block_mut(*old_id);
            let mut instructions = mem::take(&mut seq.instrs);
            for (instr, _) in &mut instructions {
                match instr {
                    ir::Instr::Block(ir::Block { seq })
                    | ir::Instr::Loop(ir::Loop { seq })
                    | ir::Instr::Br(ir::Br { block: seq })
                    | ir::Instr::BrIf(ir::BrIf { block: seq }) => {
                        *seq = self.sequence_mapping[seq];
                    }

                    ir::Instr::IfElse(ir::IfElse {
                        consequent,
                        alternative,
                    }) => {
                        *consequent = self.sequence_mapping[consequent];
                        *alternative = self.sequence_mapping[alternative];
                    }
                    ir::Instr::BrTable(ir::BrTable { blocks, default }) => {
                        for block in blocks.iter_mut() {
                            *block = self.sequence_mapping[block];
                        }
                        *default = self.sequence_mapping[default];
                    }

                    ir::Instr::LocalGet(ir::LocalGet { local }) => {
                        if let Some(new_local) = replacer.take_replacement(*old_id, *local) {
                            *local = new_local;
                        }
                    }

                    _ => { /* Do nothing */ }
                }
            }

            *builder.instr_seq(*new_id).instrs_mut() = instructions;
        }

        *local_fn.builder_mut() = builder;
    }
}

impl ir::Visitor<'_> for FunctionCloner {
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
            module: fn_module(&function.kind).map(str::to_owned),
            name: function.name.to_owned(),
            expected_arity: function.externrefs.bit_len(),
            real_arity: params.len() + results.len(),
        });
    }

    let mut new_params = params.to_vec();
    let mut new_results = results.to_vec();
    for idx in function.externrefs.set_indices() {
        let placement = if idx < new_params.len() {
            &mut new_params[idx]
        } else {
            &mut new_results[idx - new_params.len()]
        };

        if *placement != ValType::I32 {
            return Err(Error::UnexpectedType {
                module: fn_module(&function.kind).map(str::to_owned),
                name: function.name.to_owned(),
                location: if idx < new_params.len() {
                    Location::Arg(idx)
                } else {
                    Location::ReturnType(idx - new_params.len())
                },
                real_type: new_params[idx],
            });
        }
        *placement = ValType::Externref;
    }

    #[cfg(feature = "processor-log")]
    log::debug!(
        target: "externref",
        "Replacing signature {:?} -> {:?} with {:?} -> {:?}",
        params, results, new_params, new_results
    );
    Ok((new_params, new_results))
}

fn fn_module<'a>(fn_kind: &FunctionKind<'a>) -> Option<&'a str> {
    match fn_kind {
        FunctionKind::Export => None,
        FunctionKind::Import(module) => Some(*module),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detecting_calls_to_functions_returning_ref() {
        const MODULE_BYTES: &[u8] = br#"
            (module
                (import "test" "function" (func $get_ref (result i32)))

                (func (export "test") (param $ref i32)
                    (local $x i32)
                    (local.set $x (local.get $ref)) ;; new local not required
                    (local.set $x (call $get_ref)) ;; new local required
                    (drop (local.get $x)) ;; new local used
                    (drop (local.tee $x (local.get $ref))) ;; existing local $x should be used
                    (drop (local.get $x))
                    (drop (call $get_ref))
                )
            )
        "#;

        let module = wat::parse_bytes(MODULE_BYTES).unwrap();
        let mut module = Module::from_buffer(&module).unwrap();
        let functions_returning_ref: HashSet<_> = module
            .funcs
            .iter()
            .filter_map(|function| {
                if matches!(&function.kind, walrus::FunctionKind::Import(_)) {
                    Some(function.id())
                } else {
                    None
                }
            })
            .collect();

        let fn_id = module.exports.iter().find_map(|export| {
            if export.name == "test" {
                Some(export.item)
            } else {
                None
            }
        });
        let fn_id = match fn_id.unwrap() {
            ExportItem::Function(fn_id) => fn_id,
            _ => unreachable!(),
        };

        ProcessingState::transform_local_fn(&mut module, &functions_returning_ref, fn_id);

        let ref_locals: Vec<_> = module
            .locals
            .iter()
            .filter(|local| local.ty() == ValType::Externref)
            .collect();
        assert_eq!(ref_locals.len(), 1, "{:?}", ref_locals);
        let ref_local_id = ref_locals[0].id();

        let local_fn = module.funcs.get(fn_id).kind.unwrap_local();
        let mut mentions = LocalMentions::default();
        ir::dfs_in_order(&mut mentions, local_fn, local_fn.entry_block());
        assert_eq!(mentions.local_counts[&ref_local_id], 2);
    }

    #[derive(Debug, Default)]
    struct LocalMentions {
        local_counts: HashMap<LocalId, usize>,
    }

    impl ir::Visitor<'_> for LocalMentions {
        fn visit_local_id(&mut self, local_id: &LocalId) {
            *self.local_counts.entry(*local_id).or_default() += 1;
        }
    }
}
