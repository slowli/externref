use assert_matches::assert_matches;
use once_cell::sync::Lazy;
use tracing_subscriber::{fmt::format::FmtSpan, FmtSubscriber};
use wasmtime::{Caller, Engine, Extern, ExternRef, Linker, Module, Store, Table, Trap, Val};

use std::{collections::HashSet, sync::Once};

use externref::processor::Processor;

mod compile;

use crate::compile::compile;

type RefAssertion = fn(Caller<'_, Data>, &Table);

static OPTIMIZED_MODULE: Lazy<Vec<u8>> = Lazy::new(|| compile(true));

fn enable_tracing() {
    static TRACING: Once = Once::new();

    TRACING.call_once(|| {
        FmtSubscriber::builder()
            .pretty()
            .with_span_events(FmtSpan::CLOSE)
            .with_test_writer()
            .with_env_filter("externref=debug")
            .init();
    });
}

#[derive(Debug)]
struct HostSender {
    key: String,
}

struct Data {
    externrefs: Option<Table>,
    ref_assertions: Vec<RefAssertion>,
    senders: HashSet<String>,
    dropped: Vec<ExternRef>,
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

    fn assert_drops(&self, expected_strings: &[&str]) {
        let dropped_strings = self
            .dropped
            .iter()
            .filter_map(|drop| drop.data().downcast_ref::<Box<str>>().map(AsRef::as_ref));
        let dropped_strings: Vec<&str> = dropped_strings.collect();
        assert_eq!(dropped_strings, *expected_strings);
    }
}

fn send_message(
    mut ctx: Caller<'_, Data>,
    resource: Option<ExternRef>,
    buffer_ptr: u32,
    buffer_len: u32,
) -> Result<Option<ExternRef>, Trap> {
    let memory = ctx
        .get_export("memory")
        .and_then(Extern::into_memory)
        .ok_or_else(|| Trap::new("module memory is not exposed"))?;

    let mut buffer = vec![0_u8; buffer_len as usize];
    memory
        .read(&ctx, buffer_ptr as usize, &mut buffer)
        .map_err(|err| Trap::new(format!("failed reading WASM memory: {}", err)))?;
    let buffer = String::from_utf8(buffer)
        .map_err(|err| Trap::new(format!("buffer is not utf-8: {}", err)))?;

    let resource = resource.ok_or_else(|| Trap::new("null reference passed to host"))?;
    let sender = resource
        .data()
        .downcast_ref::<HostSender>()
        .ok_or_else(|| Trap::new("passed reference has incorrect type"))?;
    assert!(ctx.data().senders.contains(&sender.key));

    let bytes = Box::<str>::from(buffer);
    Ok(Some(ExternRef::new(bytes)))
}

fn message_len(resource: Option<ExternRef>) -> Result<u32, Trap> {
    if let Some(resource) = resource {
        let str = resource
            .data()
            .downcast_ref::<Box<str>>()
            .ok_or_else(|| Trap::new("passed reference has incorrect type"))?;
        Ok(u32::try_from(str.len()).unwrap())
    } else {
        Ok(0)
    }
}

fn inspect_refs(mut ctx: Caller<'_, Data>) {
    let refs = ctx.data().externrefs.unwrap();
    let assertions = ctx.data_mut().ref_assertions.pop().unwrap();
    assertions(ctx, &refs);
}

fn assert_refs(mut ctx: Caller<'_, Data>, table: &Table, buffers_liveness: &[bool]) {
    let size = table.size(&ctx);
    assert_eq!(size, 1 + buffers_liveness.len() as u32);
    let refs: Vec<_> = (0..size)
        .map(|idx| table.get(&mut ctx, idx).unwrap().unwrap_externref())
        .collect();

    let sender_ref = refs[0].as_ref().unwrap();
    assert!(sender_ref.data().is::<HostSender>());

    for (buffer_ref, &live) in refs[1..].iter().zip(buffers_liveness) {
        if live {
            let buffer_ref = buffer_ref.as_ref().unwrap();
            assert!(buffer_ref.data().is::<Box<str>>());
        } else {
            assert!(buffer_ref.is_none());
        }
    }
}

fn drop_ref(mut ctx: Caller<'_, Data>, dropped: Option<ExternRef>) {
    let dropped = dropped.expect("drop fn called with null ref");
    ctx.data_mut().dropped.push(dropped);
}

fn create_linker(engine: &Engine) -> Linker<Data> {
    let mut linker = Linker::new(engine);
    linker
        .func_wrap("test", "send_message", send_message)
        .unwrap();
    linker
        .func_wrap("test", "message_len", message_len)
        .unwrap();
    linker
        .func_wrap("test", "inspect_refs", inspect_refs)
        .unwrap();
    linker.func_wrap("test", "drop_ref", drop_ref).unwrap();
    linker
}

#[test]
fn transform_after_optimization() {
    enable_tracing();

    let module = Processor::default()
        .set_drop_fn("test", "drop_ref")
        .process_bytes(&OPTIMIZED_MODULE)
        .unwrap();
    let module = Module::new(&Engine::default(), &module).unwrap();
    let linker = create_linker(module.engine());

    let ref_assertions: Vec<RefAssertion> = vec![
        |caller, table| assert_refs(caller, table, &[]),
        |caller, table| assert_refs(caller, table, &[true]),
        |caller, table| assert_refs(caller, table, &[true; 2]),
        |caller, table| assert_refs(caller, table, &[true; 3]),
        |caller, table| assert_refs(caller, table, &[false, true, true]),
        |caller, table| assert_refs(caller, table, &[false; 3]),
    ];
    let mut store = Store::new(module.engine(), Data::new(ref_assertions));
    let instance = linker.instantiate(&mut store, &module).unwrap();
    let externrefs = instance.get_table(&mut store, "externrefs").unwrap();
    store.data_mut().externrefs = Some(externrefs);

    let exported_fn = instance
        .get_typed_func::<Option<ExternRef>, (), _>(&mut store, "test_export")
        .unwrap();
    let sender = store.data_mut().push_sender("sender");
    exported_fn
        .call(&mut store, Some(ExternRef::new(sender)))
        .unwrap();

    store
        .data()
        .assert_drops(&["test", "some other string", "42"]);

    store.gc();
    let size = externrefs.size(&store);
    assert_eq!(size, 4); // sender + 3 buffers
    for i in 0..size {
        assert_matches!(externrefs.get(&mut store, i).unwrap(), Val::ExternRef(None));
    }
}

#[test]
fn null_references() {
    enable_tracing();

    let module = Processor::default()
        .process_bytes(&OPTIMIZED_MODULE)
        .unwrap();
    let module = Module::new(&Engine::default(), &module).unwrap();
    let linker = create_linker(module.engine());
    let mut store = Store::new(module.engine(), Data::new(vec![]));
    let instance = linker.instantiate(&mut store, &module).unwrap();

    let test_fn = instance
        .get_typed_func::<Option<ExternRef>, (), _>(&mut store, "test_nulls")
        .unwrap();
    let sender = store.data_mut().push_sender("sender");
    test_fn
        .call(&mut store, Some(ExternRef::new(sender)))
        .unwrap();
    test_fn.call(&mut store, None).unwrap();
}
