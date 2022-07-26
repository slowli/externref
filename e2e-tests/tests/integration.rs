use assert_matches::assert_matches;
use wasmtime::{Caller, Engine, Extern, ExternRef, Linker, Module, Store, Table, Trap, Val};

use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Weak},
};

use externref_processor::process_bytes;

const WASM_PROFILE: &str = "wasm";

fn target_dir() -> PathBuf {
    let mut path = env::current_exe().expect("Cannot get path to executing test");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path
}

fn wasm_target_dir(target_dir: PathBuf) -> PathBuf {
    let mut root_dir = target_dir;
    while !root_dir.join("wasm32-unknown-unknown").is_dir() {
        assert!(
            root_dir.pop(),
            "Cannot find dir for the `wasm32-unknown-unknown` target"
        );
    }
    root_dir.join("wasm32-unknown-unknown").join(WASM_PROFILE)
}

fn compile_wasm() -> PathBuf {
    let profile = format!("--profile={}", WASM_PROFILE);
    let mut command = Command::new("cargo");
    command.args([
        "build",
        "--lib",
        "--target",
        "wasm32-unknown-unknown",
        &profile,
    ]);

    let mut command = command
        .stdin(Stdio::null())
        .spawn()
        .expect("cannot run cargo");
    let exit_status = command.wait().expect("failed waiting for cargo");
    assert!(
        exit_status.success(),
        "Compiling WASM module finished abnormally: {}",
        exit_status
    );

    let wasm_dir = wasm_target_dir(target_dir());
    let mut wasm_file = env!("CARGO_PKG_NAME").replace('-', "_");
    wasm_file.push_str(".wasm");
    wasm_dir.join(wasm_file)
}

fn optimize_wasm(wasm_file: &Path) -> PathBuf {
    let mut opt_wasm_file = PathBuf::from(wasm_file);
    opt_wasm_file.set_extension("opt.wasm");

    let mut command = Command::new("wasm-opt")
        .args(["-Os", "--enable-mutable-globals", "--strip-debug"])
        .arg("-o")
        .args([opt_wasm_file.as_ref(), wasm_file])
        .stdin(Stdio::null())
        .spawn()
        .expect("cannot run wasm-opt");

    let exit_status = command.wait().expect("failed waiting for wasm-opt");
    assert!(
        exit_status.success(),
        "Optimizing WASM module finished abnormally: {}",
        exit_status
    );
    opt_wasm_file
}

fn compile(optimize: bool) -> Vec<u8> {
    let mut wasm_file = compile_wasm();
    if optimize {
        wasm_file = optimize_wasm(&wasm_file);
    }
    fs::read(&wasm_file).unwrap_or_else(|err| {
        panic!(
            "Error reading file `{}`: {}",
            wasm_file.to_string_lossy(),
            err
        )
    })
}

type RefAssertion = fn(Caller<'_, Data>, &Table);

#[derive(Debug)]
struct HostSender {
    key: String,
}

struct Data {
    externrefs: Option<Table>,
    ref_assertions: Vec<RefAssertion>,
    senders: HashSet<String>,
    buffers: Vec<Weak<str>>,
}

impl Data {
    fn new(mut ref_assertions: Vec<fn(Caller<'_, Data>, &Table)>) -> Self {
        ref_assertions.reverse();
        Self {
            externrefs: None,
            ref_assertions,
            senders: HashSet::new(),
            buffers: vec![],
        }
    }

    fn push_sender(&mut self, name: impl Into<String>) -> HostSender {
        let name = name.into();
        self.senders.insert(name.clone());
        HostSender { key: name }
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

    let bytes = Arc::<str>::from(buffer);
    ctx.data_mut().buffers.push(Arc::downgrade(&bytes));
    Ok(Some(ExternRef::new(bytes)))
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
            assert!(buffer_ref.data().is::<Arc<str>>());
        } else {
            assert!(buffer_ref.is_none());
        }
    }
}

#[test]
fn transform_after_optimization() {
    let module = compile(true);
    let module = process_bytes(&module).unwrap();
    let module = Module::new(&Engine::default(), &module).unwrap();

    let mut linker = Linker::new(module.engine());
    linker
        .func_wrap("test", "send_message", send_message)
        .unwrap();
    linker
        .func_wrap("test", "inspect_refs", inspect_refs)
        .unwrap();

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

    store.gc();
    let size = externrefs.size(&store);
    assert_eq!(size, 4); // sender + 3 buffers
    for i in 0..size {
        assert_matches!(externrefs.get(&mut store, i).unwrap(), Val::ExternRef(None));
    }
}
