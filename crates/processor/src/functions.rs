//! Patched functions for working with `externref`s.

use walrus::{
    ir::{self, BinaryOp},
    FunctionBuilder, FunctionId, FunctionKind as WasmFunctionKind, ImportKind, InstrSeqBuilder,
    LocalId, Module, ModuleImports, TableId, ValType,
};

use std::collections::HashMap;

use crate::{Error, Processor};

#[derive(Debug)]
pub(crate) struct ExternrefImports {
    insert: Option<FunctionId>,
    get: Option<FunctionId>,
    drop: Option<FunctionId>,
}

impl ExternrefImports {
    const MODULE_NAME: &'static str = "externref";

    pub fn new(imports: &mut ModuleImports) -> Result<Self, Error> {
        Ok(Self {
            insert: Self::take_import(imports, "insert")?,
            get: Self::take_import(imports, "get")?,
            drop: Self::take_import(imports, "drop")?,
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
}

impl PatchedFunctions {
    pub fn new(module: &mut Module, imports: &ExternrefImports, processor: &Processor<'_>) -> Self {
        let table_id = module.tables.add_local(0, None, ValType::Externref);
        module.exports.add(processor.table_name, table_id);

        let mut fn_mapping = HashMap::with_capacity(3);
        if let Some(fn_id) = imports.insert {
            #[cfg(feature = "log")]
            log::debug!(
                target: "externref",
                "Need to replace `externref::insert` import (storing an externref in table)"
            );

            module.funcs.delete(fn_id);
            fn_mapping.insert(fn_id, Self::patch_insert_fn(module, table_id));
        }
        if let Some(fn_id) = imports.get {
            #[cfg(feature = "log")]
            log::debug!(
                target: "externref",
                "Need to replace `externref::get` import (getting an externref from table)"
            );

            module.funcs.delete(fn_id);
            fn_mapping.insert(fn_id, Self::patch_get_fn(module, table_id));
        }
        if let Some(fn_id) = imports.drop {
            #[cfg(feature = "log")]
            log::debug!(
                target: "externref",
                "Need to replace `externref::drop` import (dropping an externref from table)"
            );

            module.funcs.delete(fn_id);
            let drop_fn_id = processor.drop_fn_name.map(|(module_name, name)| {
                let ty = module.types.add(&[ValType::Externref], &[]);
                module.add_import_func(module_name, name, ty).0
            });
            fn_mapping.insert(fn_id, Self::patch_drop_fn(module, table_id, drop_fn_id));
        }
        Self { fn_mapping }
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
                        .local_set(free_idx) // == 1
                        .block(None, |loop_wrapper| {
                            Self::create_loop(loop_wrapper, table_id, free_idx);
                        });
                },
                |_| {},
            )
            .local_get(free_idx) // == 0
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

    pub fn replace_calls(&self, module: &mut Module) {
        #[cfg(feature = "log")]
        log::debug!(target: "externref", "Replacing calls to externref imports...");

        let mut visitor = ReplaceFunctions::new(&self.fn_mapping);
        for function in module.funcs.iter_mut() {
            if let WasmFunctionKind::Local(local_fn) = &mut function.kind {
                ir::dfs_pre_order_mut(&mut visitor, local_fn, local_fn.entry_block());
            }
        }
        #[cfg(feature = "log")]
        log::info!(
            target: "externref",
            "Replaced {} calls to externref imports",
            visitor.replaced_count
        );
    }
}

/// Visitor replacing invocations of patched functions.
#[derive(Debug)]
struct ReplaceFunctions<'a> {
    fn_mapping: &'a HashMap<FunctionId, FunctionId>,
    replaced_count: usize,
}

impl<'a> ReplaceFunctions<'a> {
    fn new(fn_mapping: &'a HashMap<FunctionId, FunctionId>) -> Self {
        Self {
            fn_mapping,
            replaced_count: 0,
        }
    }
}

impl ir::VisitorMut for ReplaceFunctions<'_> {
    fn visit_function_id_mut(&mut self, function: &mut FunctionId) {
        if let Some(mapped_id) = self.fn_mapping.get(function) {
            *function = *mapped_id;
            self.replaced_count += 1;
        }
    }
}
