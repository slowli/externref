//! WASM module compilation logic.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

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

    pub fn compile(self) -> Vec<u8> {
        let mut wasm_file = compile_wasm(self.rust_profile());
        if matches!(self, Self::OptimizedWasm) {
            wasm_file = optimize_wasm(&wasm_file);
        }
        fs::read(&wasm_file).unwrap_or_else(|err| {
            panic!(
                "Error reading file `{}`: {err}",
                wasm_file.to_string_lossy()
            )
        })
    }
}
