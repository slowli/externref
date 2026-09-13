//! End-to-end tests for the `externref` macro / processor.

use std::{
    collections::HashSet,
    sync::{LazyLock, Once},
};

use assert_matches::assert_matches;
use test_casing::{Product, test_casing};
use tracing::{Level, Subscriber, subscriber::DefaultGuard};
use tracing_capture::{CaptureLayer, SharedStorage, Storage};
use tracing_subscriber::{
    FmtSubscriber, fmt::format::FmtSpan, layer::SubscriberExt, registry::LookupSpan,
};
use wasmtime::{
    AsContextMut, Caller, Engine, Extern, ExternRef, Instance, Linker, Module, OwnedRooted, Ref,
    Rooted, Store, Table, error::Context as _,
};

use crate::compile::{CompilationProfile, CompiledModule};

mod compile;

type RefAssertion = fn(Caller<'_, Data>, &Table);

fn compile_module(profile: CompilationProfile) -> &'static CompiledModule {
    static UNOPTIMIZED_MODULE: LazyLock<CompiledModule> =
        LazyLock::new(|| CompilationProfile::Wasm.compile());
    static OPTIMIZED_MODULE: LazyLock<CompiledModule> =
        LazyLock::new(|| CompilationProfile::OptimizedWasm.compile());
    static DEBUG_MODULE: LazyLock<CompiledModule> =
        LazyLock::new(|| CompilationProfile::Debug.compile());
    static RELEASE_MODULE: LazyLock<CompiledModule> =
        LazyLock::new(|| CompilationProfile::Release.compile());

    match profile {
        CompilationProfile::Wasm => &UNOPTIMIZED_MODULE,
        CompilationProfile::OptimizedWasm => &OPTIMIZED_MODULE,
        CompilationProfile::Debug => &DEBUG_MODULE,
        CompilationProfile::Release => &RELEASE_MODULE,
    }
}

fn create_fmt_subscriber() -> impl Subscriber + for<'a> LookupSpan<'a> {
    FmtSubscriber::builder()
        .pretty()
        .with_span_events(FmtSpan::CLOSE)
        .with_test_writer()
        .with_env_filter("info,externref=debug")
        .finish()
}

fn enable_tracing() {
    static TRACING: Once = Once::new();

    TRACING.call_once(|| {
        tracing::subscriber::set_global_default(create_fmt_subscriber()).ok();
    });
}

fn enable_tracing_assertions() -> (DefaultGuard, SharedStorage) {
    let storage = SharedStorage::default();
    let subscriber = create_fmt_subscriber().with(CaptureLayer::new(&storage));
    let guard = tracing::subscriber::set_default(subscriber);
    (guard, storage)
}

#[derive(Debug)]
struct HostSender {
    key: String,
}

struct Data {
    externrefs: Option<Table>,
    ref_assertions: Vec<RefAssertion>,
    senders: HashSet<String>,
    dropped: Vec<OwnedRooted<ExternRef>>,
}

impl Data {
    fn new(mut ref_assertions: Vec<fn(Caller<'_, Data>, &Table)>) -> Self {
        ref_assertions.reverse();
        Self {
            externrefs: None,
            ref_assertions,
            senders: HashSet::new(),
            dropped: vec![],
        }
    }

    fn push_sender(&mut self, name: impl Into<String>) -> HostSender {
        let name = name.into();
        self.senders.insert(name.clone());
        HostSender { key: name }
    }

    fn assert_drops(&self, store: &Store<Data>, expected_strings: &HashSet<&str>) {
        let dropped_strings = self.dropped.iter().filter_map(|drop| {
            drop.data(store)
                .expect("reference was unexpectedly garbage-collected")
                .unwrap()
                .downcast_ref::<Box<str>>()
                .map(AsRef::as_ref)
        });
        let dropped_strings: HashSet<&str> = dropped_strings.collect();
        assert_eq!(dropped_strings, *expected_strings);
    }
}

fn send_message(
    mut ctx: Caller<'_, Data>,
    resource: Rooted<ExternRef>,
    buffer_ptr: u32,
    buffer_len: u32,
) -> wasmtime::Result<Rooted<ExternRef>> {
    let memory = ctx
        .get_export("memory")
        .and_then(Extern::into_memory)
        .ok_or_else(|| wasmtime::format_err!("module memory is not exposed"))?;

    let mut buffer = vec![0_u8; buffer_len as usize];
    memory
        .read(&ctx, buffer_ptr as usize, &mut buffer)
        .context("failed reading WASM memory")?;
    let buffer = String::from_utf8(buffer).context("buffer is not utf-8")?;

    let sender = resource
        .data(&ctx)?
        .context("null reference")?
        .downcast_ref::<HostSender>()
        .ok_or_else(|| wasmtime::format_err!("passed reference has incorrect type"))?;
    assert!(ctx.data().senders.contains(&sender.key));

    let bytes = Box::<str>::from(buffer);
    ExternRef::new(&mut ctx, bytes)
}

fn message_len(
    ctx: Caller<'_, Data>,
    resource: Option<Rooted<ExternRef>>,
) -> wasmtime::Result<u32> {
    let Some(resource) = resource else {
        return Ok(0);
    };
    let str = resource
        .data(&ctx)
        .context("passed reference is garbage-collected")?
        .context("null reference")?
        .downcast_ref::<Box<str>>()
        .context("passed reference has incorrect type")?;
    Ok(u32::try_from(str.len()).unwrap())
}

fn inspect_refs(mut ctx: Caller<'_, Data>) {
    let refs = ctx.data().externrefs.unwrap();
    let assertions = ctx.data_mut().ref_assertions.pop().unwrap();
    assertions(ctx, &refs);
}

#[tracing::instrument(skip(ctx))]
fn inspect_message(mut ctx: Caller<'_, Data>, ref_idx: u32) {
    inspect_inner(&mut ctx, ref_idx.into());
}

fn inspect_inner(ctx: &mut Caller<'_, Data>, ref_idx: u64) {
    let refs_table = ctx.data().externrefs.unwrap();
    let size = refs_table.size(&ctx);
    assert!(ref_idx < size, "size={size}, ref_idx={ref_idx}");

    let buffer_ref = refs_table.get(&mut *ctx, ref_idx).unwrap();
    let buffer_ref = buffer_ref.unwrap_extern().unwrap();
    assert!(buffer_ref.data(&ctx).unwrap().unwrap().is::<Box<str>>());
}

#[tracing::instrument(skip(ctx))]
fn inspect_message_ref(mut ctx: Caller<'_, Data>, resource_ptr: u32) {
    let memory = ctx.get_export("memory").unwrap().into_memory().unwrap();
    let mut buffer = [0_u8; 4];
    memory
        .read(&ctx, resource_ptr as usize, &mut buffer)
        .unwrap();

    // We know conversion to an index will work due to `repr(C)` on `Resource`.
    let ref_idx = u64::from(u32::from_le_bytes(buffer));
    tracing::info!(ref_idx, "read `Resource` data");

    inspect_inner(&mut ctx, ref_idx);
}

fn assert_refs(
    mut ctx: impl AsContextMut,
    table: &Table,
    alive_sender: bool,
    buffers_liveness: &[bool],
) {
    let size = table.size(&ctx);
    assert_eq!(size, 1 + buffers_liveness.len() as u64);
    let refs: Vec<_> = (0..size)
        .map(|idx| table.get(&mut ctx, idx).unwrap())
        .collect();
    let refs: Vec<_> = refs.iter().map(Ref::unwrap_extern).collect();

    let sender_ref = refs[0].as_ref();
    if alive_sender {
        let sender_ref = sender_ref.expect("sender dropped");
        assert!(sender_ref.data(&ctx).unwrap().unwrap().is::<HostSender>());
    } else {
        assert!(sender_ref.is_none());
    }

    for (buffer_ref, &live) in refs[1..].iter().zip(buffers_liveness) {
        if live {
            let buffer_ref = buffer_ref.as_ref().unwrap();
            assert!(buffer_ref.data(&ctx).unwrap().unwrap().is::<Box<str>>());
        } else {
            assert!(buffer_ref.is_none());
        }
    }
}

fn drop_ref(mut ctx: Caller<'_, Data>, dropped: Option<Rooted<ExternRef>>) {
    let dropped = dropped.expect("drop fn called with null ref");
    let dropped = dropped.to_owned_rooted(&mut ctx).unwrap();
    ctx.data_mut().dropped.push(dropped);
}

fn create_linker(engine: &Engine) -> Linker<Data> {
    let mut linker = Linker::new(engine);
    linker
        .func_wrap("test", "send_message", send_message)
        .unwrap();
    linker
        .func_wrap("test", "send_message_copy", send_message)
        .unwrap();
    linker
        .func_wrap("test", "send_message_non_null", send_message)
        .unwrap();
    linker
        .func_wrap(
            "test",
            "round_trip",
            |_value: Rooted<ExternRef>,
             _reference: Rooted<ExternRef>,
             _mut_reference: Rooted<ExternRef>,
             optional_value: Option<Rooted<ExternRef>>,
             optional_reference: Option<Rooted<ExternRef>>,
             optional_mut_reference: Option<Rooted<ExternRef>>| {
                assert_eq!(optional_value.is_some(), optional_reference.is_some());
                assert_eq!(optional_value.is_some(), optional_mut_reference.is_some());
                optional_value
            },
        )
        .unwrap();
    linker
        .func_wrap(
            "wasm:js-string",
            "cast",
            |ctx: Caller<'_, Data>, resource: Option<Rooted<ExternRef>>| {
                let resource = resource.context("null string")?;
                let data = resource.data(&ctx)?.context("null reference")?;
                if !data.is::<Box<str>>() {
                    return Err(wasmtime::format_err!("not a string"));
                }
                Ok(resource)
            },
        )
        .unwrap();
    linker
        .func_wrap("test", "message_len", message_len)
        .unwrap();
    linker
        .func_wrap("test", "inspect_refs", inspect_refs)
        .unwrap();
    linker
        .func_wrap("test", "inspect_message", inspect_message)
        .unwrap();
    linker
        .func_wrap("test", "inspect_message_ref", inspect_message_ref)
        .unwrap();
    linker.func_wrap("test", "drop_ref", drop_ref).unwrap();
    linker
}

#[test_casing(8, Product((CompilationProfile::ALL, ["test_export", "test_export_with_casts"])))]
fn transform_module(profile: CompilationProfile, test_export: &str) {
    let (_guard, storage) = enable_tracing_assertions();

    let module = compile_module(profile).process();
    let module = Module::new(&Engine::default(), module).unwrap();
    let linker = create_linker(module.engine());

    assert_tracing_output(&storage.lock());

    let ref_assertions: Vec<RefAssertion> = vec![
        |caller, table| assert_refs(caller, table, true, &[]),
        |caller, table| assert_refs(caller, table, true, &[true]),
        |caller, table| assert_refs(caller, table, true, &[true; 2]),
        |caller, table| assert_refs(caller, table, true, &[true; 3]),
        |caller, table| assert_refs(caller, table, true, &[false, true, true]),
        |caller, table| assert_refs(caller, table, true, &[false, true, true]),
        |caller, table| assert_refs(caller, table, true, &[false; 3]),
    ];
    let mut store = Store::new(module.engine(), Data::new(ref_assertions));
    let instance = linker.instantiate(&mut store, &module).unwrap();
    let externrefs = instance.get_table(&mut store, "externrefs").unwrap();
    store.data_mut().externrefs = Some(externrefs);

    let exported_fn = instance
        .get_typed_func::<Rooted<ExternRef>, ()>(&mut store, test_export)
        .unwrap();
    let sender = store.data_mut().push_sender("sender");
    let sender = ExternRef::new(&mut store, sender).unwrap();
    exported_fn.call(&mut store, sender).unwrap();

    store
        .data()
        .assert_drops(&store, &["test", "some other string", "42"].into());

    store.gc(None).unwrap();
    let size = externrefs.size(&store);
    assert_eq!(size, 4); // sender + 3 buffers
    for i in 0..size {
        assert_matches!(externrefs.get(&mut store, i).unwrap(), Ref::Extern(None));
    }
}

fn assert_tracing_output(storage: &Storage) {
    use predicates::{
        ord::{eq, gt},
        str::contains,
    };
    use tracing_capture::predicates::{ScanExt, field, into_fn, level, message, name, value};

    let spans = storage.scan_spans();
    let process_span = spans.single(&name(eq("process")));
    let matches =
        level(Level::INFO) & message(eq("parsed custom section")) & field("functions.len", 15_u64);
    process_span.scan_events().single(&matches);

    let patch_imports_span = spans.single(&name(eq("patch_imports")));
    let matches = into_fn(message(contains("replaced import")) & level(Level::DEBUG));
    let replaced_imports = patch_imports_span.events().filter_map(|event| {
        if matches(&event) {
            event.value("name")?.as_str()
        } else {
            None
        }
    });
    let replaced_imports: HashSet<_> = replaced_imports.collect();
    assert_eq!(
        replaced_imports,
        HashSet::from_iter(["externref::insert", "externref::get", "externref::drop"])
    );

    let replace_functions_span = spans.single(&name(eq("replace_functions")));
    let matches = level(Level::INFO)
        & message(contains("replaced calls"))
        & field("replaced_count", value(gt(0_u64)));
    replace_functions_span.scan_events().single(&matches);

    let transformed_imports = storage.all_spans().filter_map(|span| {
        if span.metadata().name() == "transform_import" {
            Some((
                span.value("module")?.as_str()?,
                span.value("name")?.as_str()?,
            ))
        } else {
            None
        }
    });
    let transformed_imports: HashSet<_> = transformed_imports.collect();
    assert_eq!(
        transformed_imports,
        HashSet::from_iter([
            ("test", "send_message"),
            ("test", "send_message_copy"),
            ("test", "send_message_non_null"),
            ("test", "message_len"),
            ("test", "round_trip"),
            ("wasm:js-string", "cast"),
        ])
    );

    let transformed_exports = storage.all_spans().filter_map(|span| {
        if span.metadata().name() == "transform_export" {
            span.value("name")?.as_str()
        } else {
            None
        }
    });
    let transformed_exports: HashSet<_> = transformed_exports.collect();
    assert!(
        transformed_exports.contains("test_nulls"),
        "{transformed_exports:?}"
    );

    // Since `test_export` and `test_export_with_casts` have the same logic, they may be optimized
    // to a single implementation.
    let contains_export = transformed_exports.contains("test_export");
    let contains_export_with_casts = transformed_exports.contains("test_export_with_casts");
    assert!(
        contains_export || contains_export_with_casts,
        "{transformed_exports:?}"
    );
    assert_eq!(
        transformed_exports.len(),
        7 + contains_export as usize + contains_export_with_casts as usize,
        "{transformed_exports:?}"
    );
}

fn init_sender(profile: CompilationProfile) -> (Instance, Store<Data>, Rooted<ExternRef>) {
    let module = compile_module(profile).process();
    let module = Module::new(&Engine::default(), module).unwrap();
    let linker = create_linker(module.engine());
    let mut store = Store::new(module.engine(), Data::new(vec![]));
    let instance = linker.instantiate(&mut store, &module).unwrap();

    let sender = store.data_mut().push_sender("sender");
    let sender = ExternRef::new(&mut store, sender).unwrap();
    (instance, store, sender)
}

#[test_casing(4, CompilationProfile::ALL)]
fn null_references(profile: CompilationProfile) {
    enable_tracing();

    let (instance, mut store, sender) = init_sender(profile);
    let test_fn = instance
        .get_typed_func::<Option<Rooted<ExternRef>>, ()>(&mut store, "test_nulls")
        .unwrap();
    test_fn.call(&mut store, Some(sender)).unwrap();
    test_fn.call(&mut store, None).unwrap();
}

#[test_casing(4, CompilationProfile::ALL)]
fn returning_resource_from_guest(profile: CompilationProfile) {
    enable_tracing();

    let (instance, mut store, sender) = init_sender(profile);
    let test_fn = instance
        .get_typed_func::<Option<Rooted<ExternRef>>, Rooted<ExternRef>>(
            &mut store,
            "test_returning_resource",
        )
        .unwrap();
    let returned_sender = test_fn.call(&mut store, Some(sender)).unwrap();

    let returned_sender = returned_sender.data(&store).unwrap().expect("no data");
    let returned_sender = returned_sender.downcast_ref::<HostSender>().unwrap();
    assert_eq!(returned_sender.key, "sender");

    let externrefs = instance.get_table(&mut store, "externrefs").unwrap();
    assert_refs(&mut store, &externrefs, false, &[false]);

    let dropped = &store.data().dropped;
    assert_eq!(dropped.len(), 2);
    let dropped: Vec<_> = dropped
        .iter()
        .map(|drop| {
            drop.data(&store)
                .expect("reference was unexpectedly garbage-collected")
                .unwrap()
        })
        .collect();
    // The buffer should be dropped first, then the sender
    assert!(dropped[0].is::<Box<str>>());
    assert!(dropped[1].is::<HostSender>());
}

#[test_casing(4, CompilationProfile::ALL)]
fn non_null_imports_and_exports(profile: CompilationProfile) {
    enable_tracing();

    let (instance, mut store, sender) = init_sender(profile);
    let function_type = instance
        .get_func(&mut store, "test_non_null")
        .unwrap()
        .ty(&store);
    assert!(matches!(
        function_type.params().next().unwrap(),
        wasmtime::ValType::Ref(ref_type) if !ref_type.is_nullable()
    ));
    assert!(matches!(
        function_type.results().next().unwrap(),
        wasmtime::ValType::Ref(ref_type) if !ref_type.is_nullable()
    ));

    let test_fn = instance
        .get_typed_func::<Rooted<ExternRef>, Rooted<ExternRef>>(&mut store, "test_non_null")
        .unwrap();
    let returned = test_fn.call(&mut store, sender).unwrap();
    let returned = returned.data(&store).unwrap().unwrap();
    assert_eq!(returned.downcast_ref::<HostSender>().unwrap().key, "sender");

    let externrefs = instance.get_table(&mut store, "externrefs").unwrap();
    assert_refs(&mut store, &externrefs, false, &[false]);
    assert_eq!(store.data().dropped.len(), 2);
    store.data().assert_drops(&store, &["non-null"].into());
}

#[test_casing(4, CompilationProfile::ALL)]
fn non_null_js_string_cast(profile: CompilationProfile) {
    enable_tracing();

    let (instance, mut store, sender) = init_sender(profile);
    let test_fn = instance
        .get_typed_func::<Option<Rooted<ExternRef>>, Rooted<ExternRef>>(
            &mut store,
            "test_js_string_cast",
        )
        .unwrap();
    let string = ExternRef::new(&mut store, Box::<str>::from("js-string")).unwrap();
    let returned = test_fn.call(&mut store, Some(string)).unwrap();
    let returned = returned.data(&store).unwrap().unwrap();
    assert_eq!(
        returned.downcast_ref::<Box<str>>().unwrap().as_ref(),
        "js-string"
    );

    let externrefs = instance.get_table(&mut store, "externrefs").unwrap();
    for index in 0..externrefs.size(&store) {
        assert_matches!(
            externrefs.get(&mut store, index).unwrap(),
            Ref::Extern(None)
        );
    }
    assert_eq!(store.data().dropped.len(), 2);
    store.data().assert_drops(&store, &["js-string"].into());
    assert!(test_fn.call(&mut store, None).is_err());
    assert!(test_fn.call(&mut store, Some(sender)).is_err());
}

#[test_casing(4, CompilationProfile::ALL)]
fn resource_type_nullability(profile: CompilationProfile) {
    enable_tracing();

    let (instance, mut store, sender) = init_sender(profile);
    let function_type = instance
        .get_func(&mut store, "test_resource_types")
        .unwrap()
        .ty(&store);
    let params: Vec<_> = function_type
        .params()
        .map(|ty| match ty {
            wasmtime::ValType::Ref(ref_type) => ref_type.is_nullable(),
            other => panic!("expected externref, got {other}"),
        })
        .collect();
    assert_eq!(params, [false, false, false, true, true, true]);
    assert!(matches!(
        function_type.results().next().unwrap(),
        wasmtime::ValType::Ref(ref_type) if ref_type.is_nullable()
    ));

    let test_fn = instance
        .get_typed_func::<(
            Rooted<ExternRef>,
            Rooted<ExternRef>,
            Rooted<ExternRef>,
            Option<Rooted<ExternRef>>,
            Option<Rooted<ExternRef>>,
            Option<Rooted<ExternRef>>,
        ), Option<Rooted<ExternRef>>>(&mut store, "test_resource_types")
        .unwrap();
    for optional in [Some(sender), None] {
        let returned = test_fn
            .call(
                &mut store,
                (sender, sender, sender, optional, optional, optional),
            )
            .unwrap();
        assert_eq!(returned.is_some(), optional.is_some());
        if let Some(returned) = returned {
            let data = returned.data(&store).unwrap().unwrap();
            assert_eq!(data.downcast_ref::<HostSender>().unwrap().key, "sender");
        }

        let externrefs = instance.get_table(&mut store, "externrefs").unwrap();
        for index in 0..externrefs.size(&store) {
            assert_matches!(
                externrefs.get(&mut store, index).unwrap(),
                Ref::Extern(None)
            );
        }
    }
}

#[test_casing(4, CompilationProfile::ALL)]
fn resource_copies(profile: CompilationProfile) {
    enable_tracing();

    let (instance, mut store, sender) = init_sender(profile);
    let externrefs = instance.get_table(&mut store, "externrefs").unwrap();
    store.data_mut().externrefs = Some(externrefs);

    let test_fn = instance
        .get_typed_func::<Rooted<ExternRef>, ()>(&mut store, "test_export_with_copies")
        .unwrap();
    test_fn.call(&mut store, sender).unwrap();

    let externrefs = instance.get_table(&mut store, "externrefs").unwrap();
    // We allocate 2 copied buffers: one via `-> ResourceCopy` and another by leaking the resource
    assert_refs(&mut store, &externrefs, false, &[true, true]);

    // The sender is the only resource that should have been dropped
    let dropped = &store.data().dropped;
    assert_eq!(dropped.len(), 1);
    let dropped = dropped[0]
        .data(&store)
        .expect("reference was unexpectedly garbage-collected")
        .unwrap();
    assert!(dropped.is::<HostSender>());
}
