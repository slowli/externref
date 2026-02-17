//! WASM module compilation logic.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use externref::processor::Processor;

fn target_dir() -> PathBuf {
    let mut path = env::current_exe().expect("Cannot get path to executing test");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path
}

fn wasm_target_dir(target_dir: PathBuf, profile: &str) -> PathBuf {
    let mut root_dir = target_dir;
    while !root_dir.join("wasm32-unknown-unknown").is_dir() {
        assert!(
            root_dir.pop(),
            "Cannot find dir for the `wasm32-unknown-unknown` target"
        );
    }
    root_dir.join("wasm32-unknown-unknown").join(profile)
}

#[tracing::instrument(ret)]
fn compile_wasm(profile: &str) -> PathBuf {
    let profile_arg = if profile == "debug" {
        "--profile=dev".to_owned() // the debug profile has differing `--profile` and output dir naming
    } else {
        format!("--profile={profile}")
    };
    let mut command = Command::new("cargo");
    command.args([
        "build",
        "--lib",
        "--target",
        "wasm32-unknown-unknown",
        &profile_arg,
    ]);
    tracing::info!(?command, "running compilation");

    let mut command = command
        .stdin(Stdio::null())
        .spawn()
        .expect("cannot run cargo");
    let exit_status = command.wait().expect("failed waiting for cargo");
    assert!(
        exit_status.success(),
        "Compiling WASM module finished abnormally: {exit_status}"
    );

    let wasm_dir = wasm_target_dir(target_dir(), profile);
    let mut wasm_file = env!("CARGO_PKG_NAME").replace('-', "_");
    wasm_file.push_str(".wasm");
    wasm_dir.join(wasm_file)
}

#[tracing::instrument(skip_all)]
fn optimize_wasm(wasm_module: Vec<u8>, temp_dir: &Path) -> Vec<u8> {
    let module_path = temp_dir.join("in.wasm");
    fs::write(&module_path, &wasm_module).unwrap_or_else(|err| {
        panic!(
            "failed writing input module to `{}`: {err}",
            module_path.display()
        );
    });

    let output_path = temp_dir.join("out.wasm");
    let mut command = Command::new("wasm-opt");
    command
        .args(["-Os", "--enable-mutable-globals", "--strip-debug", "-o"])
        .arg(&output_path)
        .arg(&module_path)
        .stderr(Stdio::piped());
    tracing::info!(?command, "running optimization");

    let output = command
        .spawn()
        .expect("cannot run wasm-opt")
        .wait_with_output()
        .expect("failed waiting for wasm-opt");
    assert!(
        output.status.success(),
        "Optimizing WASM module finished abnormally: {exit_status}\n---- stderr ----\n{err}",
        exit_status = output.status,
        err = String::from_utf8_lossy(&output.stderr)
    );

    let output = fs::read(&output_path).unwrap_or_else(|err| {
        panic!(
            "failed reading optimized module from `{}`: {err}",
            output_path.display()
        )
    });
    tracing::info!(output.len = output.len(), "optimized module");
    output
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum CompilationProfile {
    Wasm,
    OptimizedWasm,
    Debug,
    Release,
}

impl CompilationProfile {
    pub const ALL: [Self; 4] = [Self::Wasm, Self::OptimizedWasm, Self::Debug, Self::Release];

    fn rust_profile(self) -> &'static str {
        match self {
            Self::Wasm | Self::OptimizedWasm => "wasm",
            Self::Debug => "debug",
            Self::Release => "release",
        }
    }

    #[tracing::instrument]
    pub fn compile(self) -> CompiledModule {
        let wasm_file = compile_wasm(self.rust_profile());
        let bytes = fs::read(&wasm_file).unwrap_or_else(|err| {
            panic!("Error reading file `{}`: {err}", wasm_file.display());
        });

        CompiledModule {
            bytes,
            wasmopt: matches!(self, Self::OptimizedWasm),
        }
    }
}

#[derive(Debug)]
pub(crate) struct CompiledModule {
    bytes: Vec<u8>,
    wasmopt: bool,
}

impl CompiledModule {
    pub(crate) fn process(&self) -> Vec<u8> {
        let processed_module = Processor::default()
            .set_drop_fn("test", "drop_ref")
            .process_bytes(&self.bytes)
            .unwrap();
        if self.wasmopt {
            let temp_dir = tempfile::tempdir().expect("cannot create temp dir");
            optimize_wasm(processed_module, temp_dir.path())
        } else {
            processed_module
        }
    }
}
