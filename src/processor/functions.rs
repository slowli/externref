//! Patched functions for working with `externref`s.

use walrus::{
    ir::{self, BinaryOp},
    Function, FunctionBuilder, FunctionId, FunctionKind as WasmFunctionKind, ImportKind,
    InstrLocId, InstrSeqBuilder, LocalFunction, LocalId, Module, ModuleImports, TableId, ValType,
};

use std::collections::HashSet;
use std::{cmp, collections::HashMap};

use super::{Error, Processor};

#[derive(Debug)]
pub(crate) struct ExternrefImports {
    insert: Option<FunctionId>,
    get: Option<FunctionId>,
    drop: Option<FunctionId>,
    guard: Option<FunctionId>,
}

impl ExternrefImports {
    const MODULE_NAME: &'static str = "externref";

    pub fn new(imports: &mut ModuleImports) -> Result<Self, Error> {
        Ok(Self {
            insert: Self::take_import(imports, "insert")?,
            get: Self::take_import(imports, "get")?,
            drop: Self::take_import(imports, "drop")?,
            guard: Self::take_import(imports, "guard")?,
        })
    }

    fn take_import(imports: &mut ModuleImports, name: &str) -> Result<Option<FunctionId>, Error> {
        let fn_id = imports.find(Self::MODULE_NAME, name).map(|import_id| {
            match imports.get(import_id).kind {
                ImportKind::Function(fn_id) => {
                    imports.delete(import_id);
                    Ok(fn_id)
                }
                _ => Err(Error::UnexpectedImportType {
                    module: Self::MODULE_NAME.to_owned(),
                    name: name.to_owned(),
                }),
            }
        });
        fn_id.transpose()
    }
}

#[derive(Debug)]
pub(crate) struct PatchedFunctions {
    fn_mapping: HashMap<FunctionId, FunctionId>,
    get_ref_id: Option<FunctionId>,
    guard_id: Option<FunctionId>,
}

impl PatchedFunctions {
    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(level = "debug", name = "patch_imports", skip_all)
    )]
    pub fn new(module: &mut Module, imports: &ExternrefImports, processor: &Processor<'_>) -> Self {
        let table_id = module.tables.add_local(0, None, ValType::Externref);
        if let Some(table_name) = processor.table_name {
            module.exports.add(table_name, table_id);
        }

        let mut fn_mapping = HashMap::with_capacity(3);
        let mut get_ref_id = None;

        if let Some(fn_id) = imports.insert {
            #[cfg(feature = "tracing")]
            tracing::debug!(name = "externref::insert", "replaced import");

            module.funcs.delete(fn_id);
            fn_mapping.insert(fn_id, Self::patch_insert_fn(module, table_id));
        }

        if let Some(fn_id) = imports.get {
            #[cfg(feature = "tracing")]
            tracing::debug!(name = "externref::get", "replaced import");

            module.funcs.delete(fn_id);
            let patched_fn_id = Self::patch_get_fn(module, table_id);
            fn_mapping.insert(fn_id, patched_fn_id);
            get_ref_id = Some(patched_fn_id);
        }

        if let Some(fn_id) = imports.drop {
            #[cfg(feature = "tracing")]
            tracing::debug!(name = "externref::drop", "replaced import");

            module.funcs.delete(fn_id);
            let drop_fn_id = processor.drop_fn_name.map(|(module_name, name)| {
                let ty = module.types.add(&[ValType::Externref], &[]);
                module.add_import_func(module_name, name, ty).0
            });
            fn_mapping.insert(fn_id, Self::patch_drop_fn(module, table_id, drop_fn_id));
        }

        Self {
            fn_mapping,
            get_ref_id,
            guard_id: imports.guard,
        }
    }

    // We want to implement the following logic:
    //
    // ```
    // if value == NULL {
    //     return -1;
    // }
    // let table_len = externrefs_table.len();
    // let mut free_idx;
    // if table_len > 0 {
    //     free_idx -= 1;
    //     loop {
    //         if externrefs_table[free_idx] == NULL {
    //             break;
    //         } else if free_idx == 0 {
    //             free_idx = table_len;
    //             break;
    //         } else {
    //             free_idx -= 1;
    //         }
    //     }
    // } else {
    //     free_idx = 0;
    // };
    // if free_idx == table_len {
    //     externrefs_table.grow(1, value);
    // } else {
    //     externrefs_table[free_idx] = value;
    // }
    // free_idx
    // ```
    fn patch_insert_fn(module: &mut Module, table_id: TableId) -> FunctionId {
        let mut builder =
            FunctionBuilder::new(&mut module.types, &[ValType::Externref], &[ValType::I32]);
        let value = module.locals.add(ValType::Externref);
        let free_idx = module.locals.add(ValType::I32);
        builder
            .func_body()
            .local_get(value)
            .ref_is_null()
            .if_else(
                None,
                |value_is_null| {
                    value_is_null.i32_const(-1).return_();
                },
                |_| {},
            )
            .table_size(table_id)
            .if_else(
                None,
                |table_is_not_empty| {
                    table_is_not_empty
                        .table_size(table_id)
                        .i32_const(1)
                        .binop(BinaryOp::I32Sub)
                        .local_set(free_idx)
                        .block(None, |loop_wrapper| {
                            Self::create_loop(loop_wrapper, table_id, free_idx);
                        });
                },
                |_| {},
            )
            .local_get(free_idx)
            .table_size(table_id)
            .binop(BinaryOp::I32Eq)
            .if_else(
                None,
                |growth_required| {
                    growth_required
                        .local_get(value)
                        .i32_const(1)
                        .table_grow(table_id)
                        .i32_const(-1)
                        .binop(BinaryOp::I32Eq)
                        .if_else(
                            None,
                            |growth_failed| {
                                growth_failed.unreachable();
                            },
                            |_| {},
                        );
                },
                |growth_not_required| {
                    growth_not_required
                        .local_get(free_idx)
                        .local_get(value)
                        .table_set(table_id);
                },
            )
            .local_get(free_idx);
        builder.finish(vec![value], &mut module.funcs)
    }

    fn create_loop(builder: &mut InstrSeqBuilder<'_>, table_id: TableId, free_idx: LocalId) {
        let break_id = builder.id();
        builder.loop_(None, |idx_loop| {
            let loop_id = idx_loop.id();
            idx_loop
                .local_get(free_idx)
                .table_get(table_id)
                .ref_is_null()
                .if_else(
                    None,
                    |is_null| {
                        is_null.br(break_id);
                    },
                    |is_not_null| {
                        is_not_null.local_get(free_idx).if_else(
                            None,
                            |is_not_zero| {
                                is_not_zero
                                    .local_get(free_idx)
                                    .i32_const(1)
                                    .binop(BinaryOp::I32Sub)
                                    .local_set(free_idx)
                                    .br(loop_id);
                            },
                            |is_zero| {
                                is_zero
                                    .table_size(table_id)
                                    .local_set(free_idx)
                                    .br(break_id);
                            },
                        );
                    },
                );
        });
    }

    fn patch_get_fn(module: &mut Module, table_id: TableId) -> FunctionId {
        let mut builder =
            FunctionBuilder::new(&mut module.types, &[ValType::I32], &[ValType::Externref]);
        let idx = module.locals.add(ValType::I32);
        builder
            .func_body()
            .local_get(idx)
            .i32_const(-1)
            .binop(BinaryOp::I32Eq)
            .if_else(
                ValType::Externref,
                |null_requested| {
                    null_requested.ref_null(ValType::Externref);
                },
                |elem_requested| {
                    elem_requested.local_get(idx).table_get(table_id);
                },
            );
        builder.finish(vec![idx], &mut module.funcs)
    }

    fn patch_drop_fn(
        module: &mut Module,
        table_id: TableId,
        drop_fn_id: Option<FunctionId>,
    ) -> FunctionId {
        let mut builder = FunctionBuilder::new(&mut module.types, &[ValType::I32], &[]);
        let idx = module.locals.add(ValType::I32);

        let mut instr_builder = builder.func_body();
        if let Some(drop_fn_id) = drop_fn_id {
            instr_builder
                .local_get(idx)
                .table_get(table_id)
                .call(drop_fn_id);
        }
        instr_builder
            .local_get(idx)
            .ref_null(ValType::Externref)
            .table_set(table_id);
        builder.finish(vec![idx], &mut module.funcs)
    }

    pub fn get_ref_id(&self) -> Option<FunctionId> {
        self.get_ref_id
    }

    pub fn replace_calls(
        &self,
        module: &mut Module,
    ) -> Result<(usize, HashSet<FunctionId>), Error> {
        let mut visitor = FunctionsReplacer::new(&self.fn_mapping);
        let mut guarded_fns = HashSet::new();
        for function in module.funcs.iter_mut() {
            if let WasmFunctionKind::Local(local_fn) = &mut function.kind {
                ir::dfs_pre_order_mut(&mut visitor, local_fn, local_fn.entry_block());

                if let Some(guard_id) = self.guard_id {
                    if Self::remove_guards(guard_id, function)? {
                        guarded_fns.insert(function.id());
                    }
                }
            }
        }
        Ok((visitor.replaced_count, guarded_fns))
    }

    fn remove_guards(guard_id: FunctionId, function: &mut Function) -> Result<bool, Error> {
        let local_fn = function.kind.unwrap_local_mut();
        let mut guard_visitor = GuardRemover::new(guard_id, local_fn);
        ir::dfs_pre_order_mut(&mut guard_visitor, local_fn, local_fn.entry_block());
        match guard_visitor.placement {
            None => Ok(false),
            Some(GuardPlacement::Correct) => Ok(true),
            Some(GuardPlacement::Incorrect(code_offset)) => Err(Error::IncorrectGuard {
                function_name: function.name.clone(),
                code_offset,
            }),
        }
    }
}

/// Visitor replacing invocations of patched functions.
#[derive(Debug)]
struct FunctionsReplacer<'a> {
    fn_mapping: &'a HashMap<FunctionId, FunctionId>,
    replaced_count: usize,
}

impl<'a> FunctionsReplacer<'a> {
    fn new(fn_mapping: &'a HashMap<FunctionId, FunctionId>) -> Self {
        Self {
            fn_mapping,
            replaced_count: 0,
        }
    }
}

impl ir::VisitorMut for FunctionsReplacer<'_> {
    fn visit_function_id_mut(&mut self, function: &mut FunctionId) {
        if let Some(mapped_id) = self.fn_mapping.get(function) {
            *function = *mapped_id;
            self.replaced_count += 1;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum GuardPlacement {
    Correct,
    // The encapsulated value is the WASM offset.
    Incorrect(Option<u32>),
}

/// Visitor removing invocations of a certain function.
struct GuardRemover {
    guard_id: FunctionId,
    entry_seq_id: ir::InstrSeqId,
    placement: Option<GuardPlacement>,
}

impl GuardRemover {
    fn new(guard_id: FunctionId, local_fn: &LocalFunction) -> Self {
        Self {
            guard_id,
            entry_seq_id: local_fn.entry_block(),
            placement: None,
        }
    }

    fn add_placement(&mut self, placement: GuardPlacement) {
        self.placement = cmp::max(self.placement, Some(placement));
    }
}

impl ir::VisitorMut for GuardRemover {
    fn start_instr_seq_mut(&mut self, instr_seq: &mut ir::InstrSeq) {
        let is_entry_seq = instr_seq.id() == self.entry_seq_id;
        let mut idx = 0;
        let mut maybe_set_stack_ptr = false;
        instr_seq.instrs.retain(|(instr, location)| {
            let placement = if let ir::Instr::Call(call) = instr {
                if call.func == self.guard_id {
                    Some(if is_entry_seq && (idx == 0 || maybe_set_stack_ptr) {
                        GuardPlacement::Correct
                    } else {
                        GuardPlacement::Incorrect(get_offset(*location))
                    })
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(placement) = placement {
                self.add_placement(placement);
            }
            idx += 1;
            maybe_set_stack_ptr = matches!(instr, ir::Instr::GlobalSet(_));
            placement.is_none()
        });
    }
}

/// Gets WASM bytecode offset.
pub(crate) fn get_offset(location: InstrLocId) -> Option<u32> {
    if location.is_default() {
        None
    } else {
        Some(location.data())
    }
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;

    use super::*;

    #[test]
    fn taking_externref_imports() {
        const MODULE_BYTES: &[u8] = br#"
            (module
                (import "externref" "insert" (func (param i32) (result i32)))
                (import "externref" "get" (func (param i32) (result i32)))
                (import "test" "function" (func (param f32)))
            )
        "#;

        let module = wat::parse_bytes(MODULE_BYTES).unwrap();
        let mut module = Module::from_buffer(&module).unwrap();

        let imports = ExternrefImports::new(&mut module.imports).unwrap();
        assert!(imports.insert.is_some());
        assert!(imports.get.is_some());
        assert!(imports.drop.is_none());
        assert_eq!(module.imports.iter().count(), 1);
    }

    #[test]
    fn replacing_function_calls() {
        const MODULE_BYTES: &[u8] = br#"
            (module
                (import "externref" "insert" (func $insert_ref (param i32) (result i32)))
                (import "externref" "get" (func $get_ref (param i32) (result i32)))

                (func (export "test") (param $ref i32)
                    (drop (call $get_ref
                        (call $insert_ref (local.get $ref))
                    ))
                )
            )
        "#;

        let module = wat::parse_bytes(MODULE_BYTES).unwrap();
        let mut module = Module::from_buffer(&module).unwrap();
        let imports = ExternrefImports::new(&mut module.imports).unwrap();

        let fns = PatchedFunctions::new(&mut module, &imports, &Processor::default());
        assert_eq!(fns.fn_mapping.len(), 2);
        let (replaced_calls, guarded_fns) = fns.replace_calls(&mut module).unwrap();
        assert_eq!(replaced_calls, 2); // 1 insert + 1 get
        assert!(guarded_fns.is_empty());
    }

    #[test]
    fn guarded_functions() {
        const MODULE_BYTES: &[u8] = br#"
            (module
                (import "externref" "guard" (func $guard))

                (func (param $ref i32)
                    (call $guard)
                    (drop (local.get $ref))
                )
            )
        "#;

        let module = wat::parse_bytes(MODULE_BYTES).unwrap();
        let mut module = Module::from_buffer(&module).unwrap();
        let imports = ExternrefImports::new(&mut module.imports).unwrap();

        let fns = PatchedFunctions::new(&mut module, &imports, &Processor::default());
        let (_, guarded_fns) = fns.replace_calls(&mut module).unwrap();
        assert_eq!(guarded_fns.len(), 1);
    }

    #[test]
    fn guarded_function_manipulating_stack() {
        const MODULE_BYTES: &[u8] = br#"
            (module
                (import "externref" "guard" (func $guard))
                (global $__stack_pointer (mut i32) (i32.const 32768))

                (func (param $ref i32)
                    (local $0 i32)
                    (global.set $__stack_pointer
                        (local.tee $0
                            (i32.sub (global.get $__stack_pointer) (i32.const 16))
                        )
                    )
                    (call $guard)
                    (drop (local.get $ref))
                )
            )
        "#;

        let module = wat::parse_bytes(MODULE_BYTES).unwrap();
        let mut module = Module::from_buffer(&module).unwrap();
        let imports = ExternrefImports::new(&mut module.imports).unwrap();

        let fns = PatchedFunctions::new(&mut module, &imports, &Processor::default());
        let (_, guarded_fns) = fns.replace_calls(&mut module).unwrap();
        assert_eq!(guarded_fns.len(), 1);
    }

    #[test]
    fn incorrect_guard_placement() {
        const MODULE_BYTES: &[u8] = br#"
            (module
                (import "externref" "guard" (func $guard))

                (func $test (param $ref i32)
                    (drop (local.get $ref))
                    (call $guard)
                )
            )
        "#;

        let module = wat::parse_bytes(MODULE_BYTES).unwrap();
        let mut module = Module::from_buffer(&module).unwrap();
        let imports = ExternrefImports::new(&mut module.imports).unwrap();

        let fns = PatchedFunctions::new(&mut module, &imports, &Processor::default());
        let err = fns.replace_calls(&mut module).unwrap_err();
        assert_matches!(
            err,
            Error::IncorrectGuard { function_name: Some(name), .. } if name == "test"
        );
    }
}
