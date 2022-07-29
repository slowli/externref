//! Tests for processor logic.

use walrus::{ExportItem, ImportKind, Module, RawCustomSection, ValType};

use std::path::Path;

use externref::{processor::Processor, BitSlice, Function, FunctionKind};

const ARENA_ALLOC: Function<'static> = Function {
    kind: FunctionKind::Import("arena"),
    name: "alloc",
    externrefs: BitSlice::builder::<1>(3)
        .with_set_bit(0)
        .with_set_bit(2)
        .build(),
};
const ARENA_ALLOC_BYTES: [u8; ARENA_ALLOC.custom_section_len()] = ARENA_ALLOC.custom_section();

const TEST: Function<'static> = Function {
    kind: FunctionKind::Export,
    name: "test",
    externrefs: BitSlice::builder::<1>(1).with_set_bit(0).build(),
};
const TEST_BYTES: [u8; TEST.custom_section_len()] = TEST.custom_section();

fn simple_module_path() -> &'static Path {
    Path::new("tests/modules/simple.wast")
}

fn add_basic_custom_section(module: &mut Module) {
    let mut section_data = Vec::with_capacity(ARENA_ALLOC_BYTES.len() + TEST_BYTES.len());
    section_data.extend_from_slice(&ARENA_ALLOC_BYTES);
    section_data.extend_from_slice(&TEST_BYTES);
    module.customs.add(RawCustomSection {
        name: Function::CUSTOM_SECTION_NAME.to_owned(),
        data: section_data,
    });
}

#[test]
fn basic_module() {
    let module = wat::parse_file(simple_module_path()).unwrap();
    let mut module = Module::from_buffer(&module).unwrap();
    // We need to add a custom section to the module before processing.
    add_basic_custom_section(&mut module);

    Processor::default().process(&mut module).unwrap();

    // Check that the module has the expected interface.
    assert_eq!(module.imports.iter().count(), 1, "{:?}", module.imports);
    let import_id = module.imports.find("arena", "alloc").unwrap();
    let import_id = match &module.imports.get(import_id).kind {
        ImportKind::Function(fn_id) => *fn_id,
        other => panic!("unexpected import type: {:?}", other),
    };
    let function_type = module.types.get(module.funcs.get(import_id).ty());
    assert_eq!(function_type.params(), [ValType::Externref, ValType::I32]);
    assert_eq!(function_type.results(), [ValType::Externref]);

    assert!(module.exports.iter().any(|export| {
        export.name == "externrefs" && matches!(export.item, ExportItem::Table(_))
    }));

    let export_id = module
        .exports
        .iter()
        .find_map(|export| {
            if export.name == "test" {
                Some(match &export.item {
                    ExportItem::Function(fn_id) => *fn_id,
                    other => panic!("unexpected export type: {:?}", other),
                })
            } else {
                None
            }
        })
        .unwrap();
    let function_type = module.types.get(module.funcs.get(export_id).ty());
    assert_eq!(function_type.params(), [ValType::Externref]);
    assert_eq!(function_type.results(), []);

    // Check that the module is well-formed by converting it to bytes and back.
    let module_bytes = module.emit_wasm();
    Module::from_buffer(&module_bytes).unwrap();
}

#[test]
fn basic_module_with_no_table_export_and_drop_hook() {
    let module = wat::parse_file(simple_module_path()).unwrap();
    let mut module = Module::from_buffer(&module).unwrap();
    add_basic_custom_section(&mut module);

    Processor::default()
        .set_ref_table(None)
        .set_drop_fn("hook", "drop_ref")
        .process(&mut module)
        .unwrap();

    // Check that the drop hook is imported.
    assert_eq!(module.imports.iter().count(), 2, "{:?}", module.imports);
    let import_id = module.imports.find("hook", "drop_ref").unwrap();
    let import_id = match &module.imports.get(import_id).kind {
        ImportKind::Function(fn_id) => *fn_id,
        other => panic!("unexpected import type: {:?}", other),
    };
    let function_type = module.types.get(module.funcs.get(import_id).ty());
    assert_eq!(function_type.params(), [ValType::Externref]);
    assert_eq!(function_type.results(), []);

    // Check that the refs table is not exported.
    assert!(!module
        .exports
        .iter()
        .any(|export| matches!(export.item, ExportItem::Table(_))));

    // Check that the module is well-formed by converting it to bytes and back.
    let module_bytes = module.emit_wasm();
    Module::from_buffer(&module_bytes).unwrap();
}
