//! Tests for processor logic.

use std::path::Path;

use assert_matches::assert_matches;
use externref::{
    Function, FunctionKind, TypeSlice, ValueType,
    processor::{Error, Processor},
};
use walrus::{ExportItem, ImportKind, Module, RawCustomSection, RefType, ValType};

const EXTERNREF: ValType = ValType::Ref(RefType::EXTERNREF);

const ARENA_ALLOC: Function<'static> = Function {
    kind: FunctionKind::Import("arena"),
    name: "alloc",
    types: TypeSlice::builder::<1>(3)
        .with_type(0, ValueType::NullableExternref)
        .with_type(2, ValueType::NullableExternref)
        .build(),
};
const ARENA_ALLOC_BYTES: [u8; ARENA_ALLOC.custom_section_len()] = ARENA_ALLOC.custom_section();

const TEST: Function<'static> = Function {
    kind: FunctionKind::Export,
    name: "test",
    types: TypeSlice::builder::<1>(1)
        .with_type(0, ValueType::NullableExternref)
        .build(),
};
const TEST_BYTES: [u8; TEST.custom_section_len()] = TEST.custom_section();

fn simple_module_path() -> &'static Path {
    Path::new("tests/modules/simple.wast")
}

fn no_inline_module_path() -> &'static Path {
    Path::new("tests/modules/simple-no-inline.wast")
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
fn cli_fixture_uses_current_metadata() {
    let bytes = include_bytes!("../../cli/tests/test.wasm");
    let processed = Processor::default()
        .set_drop_fn("test", "drop")
        .process_bytes(bytes)
        .unwrap();
    Module::from_buffer(&processed).unwrap();
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
        other => panic!("unexpected import type: {other:?}"),
    };
    let function_type = module.types.get(module.funcs.get(import_id).ty());
    assert_eq!(function_type.params(), [EXTERNREF, ValType::I32]);
    assert_eq!(function_type.results(), [EXTERNREF]);

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
                    other => panic!("unexpected export type: {other:?}"),
                })
            } else {
                None
            }
        })
        .unwrap();
    let function_type = module.types.get(module.funcs.get(export_id).ty());
    assert_eq!(function_type.params(), [EXTERNREF]);
    assert_eq!(function_type.results(), []);

    // Check that the module is well-formed by converting it to bytes and back.
    let module_bytes = module.emit_wasm();
    Module::from_buffer(&module_bytes).unwrap();
}

#[test]
fn non_null_import_result() {
    const NON_NULL_RESULT: Function<'static> = Function {
        kind: FunctionKind::Import("arena"),
        name: "alloc",
        types: TypeSlice::builder::<1>(3)
            .with_type(0, ValueType::NullableExternref)
            .with_type(2, ValueType::NonNullExternref)
            .build(),
    };
    const NON_NULL_BYTES: [u8; NON_NULL_RESULT.custom_section_len()] =
        NON_NULL_RESULT.custom_section();

    let module = wat::parse_file(simple_module_path()).unwrap();
    let mut module = Module::from_buffer(&module).unwrap();
    module.customs.add(RawCustomSection {
        name: Function::CUSTOM_SECTION_NAME.to_owned(),
        data: [NON_NULL_BYTES.as_slice(), TEST_BYTES.as_slice()].concat(),
    });

    Processor::default().process(&mut module).unwrap();

    let import_id = module.imports.find("arena", "alloc").unwrap();
    let ImportKind::Function(function_id) = module.imports.get(import_id).kind else {
        panic!("expected a function import");
    };
    let function_type = module.types.get(module.funcs.get(function_id).ty());
    assert_eq!(function_type.params(), [EXTERNREF, ValType::I32]);
    assert_eq!(
        function_type.results(),
        [ValType::Ref(RefType {
            nullable: false,
            ..RefType::EXTERNREF
        })]
    );
    assert!(
        module
            .customs
            .remove_raw(Function::CUSTOM_SECTION_NAME)
            .is_none()
    );

    let module_bytes = module.emit_wasm();
    Module::from_buffer(&module_bytes).unwrap();
}

#[test]
fn non_null_interfaces_do_not_add_adapters() {
    const IMPORT: Function<'static> = Function {
        kind: FunctionKind::Import("test"),
        name: "target",
        types: TypeSlice::builder::<1>(3)
            .with_type(0, ValueType::NonNullExternref)
            .with_type(2, ValueType::NonNullExternref)
            .build(),
    };
    const EXPORT: Function<'static> = Function {
        kind: FunctionKind::Export,
        name: "test",
        types: IMPORT.types,
    };
    const IMPORT_BYTES: [u8; IMPORT.custom_section_len()] = IMPORT.custom_section();
    const EXPORT_BYTES: [u8; EXPORT.custom_section_len()] = EXPORT.custom_section();
    let bytes = wat::parse_str(
        r#"
        (module
            (import "externref" "insert" (func $insert (param i32) (result i32)))
            (import "externref" "get_non_null" (func $get (param i32) (result i32)))
            (import "externref" "drop" (func $drop (param i32)))
            (import "externref" "guard" (func $guard))
            (import "test" "target" (func $target (param i32 i32) (result i32)))
            (func (export "test") (param $value i32) (param $number i32) (result i32)
                (local $index i32) (local $result i32) (local $returned i32)
                (call $guard)
                (local.set $index (call $insert (local.get $value)))
                (local.set $result (call $insert
                    (call $target (call $get (local.get $index)) (local.get $number))))
                (call $drop (local.get $index))
                (local.set $returned (call $get (local.get $result)))
                (call $drop (local.get $result))
                (local.get $returned)
            )
        )
    "#,
    )
    .unwrap();
    let mut module = Module::from_buffer(&bytes).unwrap();
    module.customs.add(RawCustomSection {
        name: Function::CUSTOM_SECTION_NAME.to_owned(),
        data: [IMPORT_BYTES.as_slice(), EXPORT_BYTES.as_slice()].concat(),
    });
    let import_id = module.imports.find("test", "target").unwrap();
    let ImportKind::Function(import_fn) = module.imports.get(import_id).kind else {
        panic!("expected a function import");
    };
    let export_id = module
        .exports
        .iter()
        .find(|export| export.name == "test")
        .unwrap()
        .id();
    let ExportItem::Function(original_export) = module.exports.get(export_id).item else {
        panic!("expected a function export");
    };

    Processor::default().process(&mut module).unwrap();

    assert_matches!(module.exports.get(export_id).item, ExportItem::Function(fn_id) if fn_id == original_export);
    assert!(matches!(
        module.funcs.get(import_fn).kind,
        walrus::FunctionKind::Import(_)
    ));
    let non_null = ValType::Ref(RefType {
        nullable: false,
        ..RefType::EXTERNREF
    });
    let ty = module.types.get(module.funcs.get(import_fn).ty());
    assert_eq!(ty.params(), [non_null, ValType::I32]);
    assert_eq!(ty.results(), [non_null]);
    let bytes = module.emit_wasm();
    Module::from_buffer(&bytes).unwrap();
}

#[test]
fn invalid_type_tag_is_rejected() {
    let module = wat::parse_file(simple_module_path()).unwrap();
    let mut module = Module::from_buffer(&module).unwrap();
    let mut data = ARENA_ALLOC_BYTES.to_vec();
    *data.last_mut().unwrap() |= 3;
    module.customs.add(RawCustomSection {
        name: Function::CUSTOM_SECTION_NAME.to_owned(),
        data,
    });

    assert_matches!(
        Processor::default().process(&mut module),
        Err(Error::Read(error)) if error.to_string().contains("invalid packed type encoding")
    );
}

#[test]
fn metadata_rejects_wrong_arity() {
    const INVALID: Function<'static> = Function {
        kind: FunctionKind::Import("arena"),
        name: "alloc",
        types: TypeSlice::builder::<1>(2)
            .with_type(0, ValueType::NonNullExternref)
            .build(),
    };
    const BYTES: [u8; INVALID.custom_section_len()] = INVALID.custom_section();

    let module = wat::parse_file(simple_module_path()).unwrap();
    let mut module = Module::from_buffer(&module).unwrap();
    module.customs.add(RawCustomSection {
        name: Function::CUSTOM_SECTION_NAME.to_owned(),
        data: [BYTES.as_slice(), TEST_BYTES.as_slice()].concat(),
    });

    assert_matches!(
        Processor::default().process(&mut module),
        Err(Error::UnexpectedArity {
            expected_arity: 2,
            real_arity: 3,
            ..
        })
    );
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
        other => panic!("unexpected import type: {other:?}"),
    };
    let function_type = module.types.get(module.funcs.get(import_id).ty());
    assert_eq!(function_type.params(), [EXTERNREF]);
    assert_eq!(function_type.results(), []);

    // Check that the refs table is not exported.
    assert!(
        !module
            .exports
            .iter()
            .any(|export| matches!(export.item, ExportItem::Table(_)))
    );

    // Check that the module is well-formed by converting it to bytes and back.
    let module_bytes = module.emit_wasm();
    Module::from_buffer(&module_bytes).unwrap();
}

#[test]
fn module_without_inlines() {
    let module = wat::parse_file(no_inline_module_path()).unwrap();
    let mut module = Module::from_buffer(&module).unwrap();
    // We need to add a custom section to the module before processing.
    add_basic_custom_section(&mut module);

    Processor::default().process(&mut module).unwrap();

    // Check that the module is well-formed by converting it to bytes and back.
    let module_bytes = module.emit_wasm();
    Module::from_buffer(&module_bytes).unwrap();
}
