//! WASM module compilation logic.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

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
    let profile = format!("--profile={WASM_PROFILE}");
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
        "Compiling WASM module finished abnormally: {exit_status}"
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
        "Optimizing WASM module finished abnormally: {exit_status}"
    );
    opt_wasm_file
}

pub fn compile(optimize: bool) -> Vec<u8> {
    let mut wasm_file = compile_wasm();
    if optimize {
        wasm_file = optimize_wasm(&wasm_file);
    }
    fs::read(&wasm_file).unwrap_or_else(|err| {
        panic!(
            "Error reading file `{}`: {err}",
            wasm_file.to_string_lossy()
        )
    })
}
